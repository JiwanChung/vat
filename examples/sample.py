"""
Data pipeline for processing clickstream events.

Reads from Kafka, transforms events, computes session-level aggregates,
and writes results to both PostgreSQL and a Parquet data lake.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import signal
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from enum import Enum, auto
from pathlib import Path
from typing import AsyncIterator, Protocol, Self

import asyncpg
import orjson
import pyarrow as pa
import pyarrow.parquet as pq
from aiokafka import AIOKafkaConsumer, AIOKafkaProducer
from prometheus_client import Counter, Histogram, start_http_server

logger = logging.getLogger(__name__)

# --- Metrics ---

EVENTS_PROCESSED = Counter(
    "pipeline_events_processed_total",
    "Total number of events processed",
    ["event_type", "status"],
)
PROCESSING_LATENCY = Histogram(
    "pipeline_processing_seconds",
    "Event processing latency in seconds",
    buckets=[0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0],
)
BATCH_SIZE = Histogram(
    "pipeline_batch_size",
    "Number of events per batch flush",
    buckets=[1, 10, 50, 100, 500, 1000],
)


# --- Domain Models ---


class EventType(Enum):
    PAGE_VIEW = auto()
    CLICK = auto()
    FORM_SUBMIT = auto()
    PURCHASE = auto()
    ERROR = auto()
    CUSTOM = auto()

    @classmethod
    def from_string(cls, value: str) -> Self:
        mapping = {
            "page_view": cls.PAGE_VIEW,
            "click": cls.CLICK,
            "form_submit": cls.FORM_SUBMIT,
            "purchase": cls.PURCHASE,
            "error": cls.ERROR,
        }
        return mapping.get(value.lower(), cls.CUSTOM)


@dataclass(frozen=True, slots=True)
class RawEvent:
    event_id: str
    user_id: str
    session_id: str
    event_type: str
    timestamp: datetime
    page_url: str
    referrer: str | None = None
    properties: dict = field(default_factory=dict)
    user_agent: str = ""
    ip_hash: str = ""

    @classmethod
    def from_kafka(cls, message: bytes) -> Self:
        data = orjson.loads(message)
        return cls(
            event_id=data["event_id"],
            user_id=data["user_id"],
            session_id=data["session_id"],
            event_type=data["event_type"],
            timestamp=datetime.fromisoformat(data["timestamp"]),
            page_url=data["page_url"],
            referrer=data.get("referrer"),
            properties=data.get("properties", {}),
            user_agent=data.get("user_agent", ""),
            ip_hash=hashlib.sha256(
                data.get("ip", "unknown").encode()
            ).hexdigest()[:16],
        )


@dataclass
class SessionAggregate:
    session_id: str
    user_id: str
    started_at: datetime
    last_activity: datetime
    page_views: int = 0
    clicks: int = 0
    purchases: int = 0
    errors: int = 0
    total_events: int = 0
    pages_visited: set[str] = field(default_factory=set)
    total_revenue_cents: int = 0
    is_bounced: bool = True

    @property
    def duration(self) -> timedelta:
        return self.last_activity - self.started_at

    @property
    def is_converted(self) -> bool:
        return self.purchases > 0

    def update(self, event: RawEvent) -> None:
        self.total_events += 1
        self.last_activity = max(self.last_activity, event.timestamp)

        event_type = EventType.from_string(event.event_type)
        match event_type:
            case EventType.PAGE_VIEW:
                self.page_views += 1
                self.pages_visited.add(event.page_url)
                if self.page_views > 1:
                    self.is_bounced = False
            case EventType.CLICK:
                self.clicks += 1
                self.is_bounced = False
            case EventType.PURCHASE:
                self.purchases += 1
                self.total_revenue_cents += event.properties.get("amount_cents", 0)
                self.is_bounced = False
            case EventType.ERROR:
                self.errors += 1
            case _:
                pass


# --- Sink Protocols ---


class EventSink(Protocol):
    async def write_batch(self, events: list[RawEvent]) -> None: ...
    async def flush_sessions(self, sessions: list[SessionAggregate]) -> None: ...
    async def close(self) -> None: ...


# --- PostgreSQL Sink ---


class PostgresSink:
    def __init__(self, dsn: str, *, batch_size: int = 500):
        self._dsn = dsn
        self._batch_size = batch_size
        self._pool: asyncpg.Pool | None = None

    async def connect(self) -> None:
        self._pool = await asyncpg.create_pool(
            self._dsn, min_size=2, max_size=10, command_timeout=30
        )
        await self._ensure_tables()

    async def _ensure_tables(self) -> None:
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute("""
                CREATE TABLE IF NOT EXISTS events (
                    event_id    TEXT PRIMARY KEY,
                    user_id     TEXT NOT NULL,
                    session_id  TEXT NOT NULL,
                    event_type  TEXT NOT NULL,
                    timestamp   TIMESTAMPTZ NOT NULL,
                    page_url    TEXT NOT NULL,
                    referrer    TEXT,
                    properties  JSONB DEFAULT '{}',
                    ip_hash     TEXT,
                    created_at  TIMESTAMPTZ DEFAULT NOW()
                );
                CREATE INDEX IF NOT EXISTS idx_events_session
                    ON events (session_id, timestamp);
                CREATE INDEX IF NOT EXISTS idx_events_user_ts
                    ON events (user_id, timestamp DESC);

                CREATE TABLE IF NOT EXISTS sessions (
                    session_id          TEXT PRIMARY KEY,
                    user_id             TEXT NOT NULL,
                    started_at          TIMESTAMPTZ NOT NULL,
                    ended_at            TIMESTAMPTZ NOT NULL,
                    duration_seconds    INTEGER NOT NULL,
                    page_views          INTEGER DEFAULT 0,
                    clicks              INTEGER DEFAULT 0,
                    purchases           INTEGER DEFAULT 0,
                    errors              INTEGER DEFAULT 0,
                    total_events        INTEGER DEFAULT 0,
                    pages_visited       TEXT[] DEFAULT '{}',
                    revenue_cents       INTEGER DEFAULT 0,
                    is_bounced          BOOLEAN DEFAULT TRUE,
                    is_converted        BOOLEAN DEFAULT FALSE,
                    updated_at          TIMESTAMPTZ DEFAULT NOW()
                );
                CREATE INDEX IF NOT EXISTS idx_sessions_user
                    ON sessions (user_id, started_at DESC);
            """)

    async def write_batch(self, events: list[RawEvent]) -> None:
        if not events or self._pool is None:
            return

        async with self._pool.acquire() as conn:
            await conn.executemany(
                """
                INSERT INTO events (event_id, user_id, session_id, event_type,
                                    timestamp, page_url, referrer, properties, ip_hash)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (event_id) DO NOTHING
                """,
                [
                    (
                        e.event_id, e.user_id, e.session_id, e.event_type,
                        e.timestamp, e.page_url, e.referrer,
                        orjson.dumps(e.properties).decode(), e.ip_hash,
                    )
                    for e in events
                ],
            )

    async def flush_sessions(self, sessions: list[SessionAggregate]) -> None:
        if not sessions or self._pool is None:
            return

        async with self._pool.acquire() as conn:
            await conn.executemany(
                """
                INSERT INTO sessions (session_id, user_id, started_at, ended_at,
                    duration_seconds, page_views, clicks, purchases, errors,
                    total_events, pages_visited, revenue_cents, is_bounced, is_converted)
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
                ON CONFLICT (session_id) DO UPDATE SET
                    ended_at = EXCLUDED.ended_at,
                    duration_seconds = EXCLUDED.duration_seconds,
                    page_views = EXCLUDED.page_views,
                    clicks = EXCLUDED.clicks,
                    purchases = EXCLUDED.purchases,
                    errors = EXCLUDED.errors,
                    total_events = EXCLUDED.total_events,
                    pages_visited = EXCLUDED.pages_visited,
                    revenue_cents = EXCLUDED.revenue_cents,
                    is_bounced = EXCLUDED.is_bounced,
                    is_converted = EXCLUDED.is_converted,
                    updated_at = NOW()
                """,
                [
                    (
                        s.session_id, s.user_id, s.started_at, s.last_activity,
                        int(s.duration.total_seconds()), s.page_views, s.clicks,
                        s.purchases, s.errors, s.total_events,
                        list(s.pages_visited), s.total_revenue_cents,
                        s.is_bounced, s.is_converted,
                    )
                    for s in sessions
                ],
            )

    async def close(self) -> None:
        if self._pool:
            await self._pool.close()


# --- Parquet / Data Lake Sink ---


class ParquetSink:
    SCHEMA = pa.schema([
        ("event_id", pa.string()),
        ("user_id", pa.string()),
        ("session_id", pa.string()),
        ("event_type", pa.string()),
        ("timestamp", pa.timestamp("us", tz="UTC")),
        ("page_url", pa.string()),
        ("referrer", pa.string()),
        ("ip_hash", pa.string()),
    ])

    def __init__(self, base_path: Path, *, partition_by: str = "date"):
        self._base_path = base_path
        self._partition_by = partition_by
        self._buffer: list[dict] = []

    async def write_batch(self, events: list[RawEvent]) -> None:
        for event in events:
            self._buffer.append({
                "event_id": event.event_id,
                "user_id": event.user_id,
                "session_id": event.session_id,
                "event_type": event.event_type,
                "timestamp": event.timestamp,
                "page_url": event.page_url,
                "referrer": event.referrer,
                "ip_hash": event.ip_hash,
            })

        if len(self._buffer) >= 10_000:
            await self._flush_to_parquet()

    async def _flush_to_parquet(self) -> None:
        if not self._buffer:
            return

        table = pa.Table.from_pylist(self._buffer, schema=self.SCHEMA)

        # Partition by date for efficient querying
        dates = defaultdict(list)
        for row in self._buffer:
            date_str = row["timestamp"].strftime("%Y-%m-%d")
            dates[date_str].append(row)

        for date_str, rows in dates.items():
            partition_table = pa.Table.from_pylist(rows, schema=self.SCHEMA)
            output_dir = self._base_path / f"date={date_str}"
            output_dir.mkdir(parents=True, exist_ok=True)

            timestamp = datetime.now(timezone.utc).strftime("%H%M%S%f")
            output_path = output_dir / f"events_{timestamp}.parquet"

            pq.write_table(
                partition_table,
                output_path,
                compression="zstd",
                row_group_size=50_000,
            )
            logger.info("Wrote %d events to %s", len(rows), output_path)

        self._buffer.clear()

    async def flush_sessions(self, sessions: list[SessionAggregate]) -> None:
        pass  # Sessions are managed in PostgreSQL only

    async def close(self) -> None:
        await self._flush_to_parquet()


# --- Pipeline ---


class Pipeline:
    SESSION_TIMEOUT = timedelta(minutes=30)

    def __init__(
        self,
        consumer: AIOKafkaConsumer,
        sinks: list[EventSink],
        *,
        batch_interval: float = 5.0,
        max_batch_size: int = 1000,
    ):
        self._consumer = consumer
        self._sinks = sinks
        self._batch_interval = batch_interval
        self._max_batch_size = max_batch_size
        self._sessions: dict[str, SessionAggregate] = {}
        self._buffer: list[RawEvent] = []
        self._running = False
        self._tasks: list[asyncio.Task] = []

    async def start(self) -> None:
        self._running = True
        await self._consumer.start()
        logger.info("Pipeline started, consuming from Kafka")

        self._tasks = [
            asyncio.create_task(self._consume_loop(), name="consume"),
            asyncio.create_task(self._flush_loop(), name="flush"),
            asyncio.create_task(self._session_gc_loop(), name="session_gc"),
        ]

        try:
            await asyncio.gather(*self._tasks)
        except asyncio.CancelledError:
            logger.info("Pipeline tasks cancelled, shutting down")

    async def stop(self) -> None:
        self._running = False
        for task in self._tasks:
            task.cancel()
        await asyncio.gather(*self._tasks, return_exceptions=True)

        # Final flush
        if self._buffer:
            await self._flush_batch()
        await self._flush_sessions(force=True)

        await self._consumer.stop()
        for sink in self._sinks:
            await sink.close()
        logger.info("Pipeline stopped gracefully")

    async def _consume_loop(self) -> None:
        async for message in self._consumer:
            if not self._running:
                break

            try:
                with PROCESSING_LATENCY.time():
                    event = RawEvent.from_kafka(message.value)
                    self._buffer.append(event)
                    self._update_session(event)
                    EVENTS_PROCESSED.labels(
                        event_type=event.event_type, status="ok"
                    ).inc()
            except Exception:
                EVENTS_PROCESSED.labels(
                    event_type="unknown", status="error"
                ).inc()
                logger.exception("Failed to process message at offset %d", message.offset)

            if len(self._buffer) >= self._max_batch_size:
                await self._flush_batch()

    async def _flush_loop(self) -> None:
        while self._running:
            await asyncio.sleep(self._batch_interval)
            if self._buffer:
                await self._flush_batch()
            await self._flush_sessions()

    async def _session_gc_loop(self) -> None:
        """Periodically expire stale sessions."""
        while self._running:
            await asyncio.sleep(60)
            now = datetime.now(timezone.utc)
            expired = [
                sid
                for sid, session in self._sessions.items()
                if now - session.last_activity > self.SESSION_TIMEOUT
            ]
            if expired:
                sessions_to_flush = [self._sessions.pop(sid) for sid in expired]
                for sink in self._sinks:
                    await sink.flush_sessions(sessions_to_flush)
                logger.info("Expired %d stale sessions", len(expired))

    def _update_session(self, event: RawEvent) -> None:
        session = self._sessions.get(event.session_id)
        if session is None:
            session = SessionAggregate(
                session_id=event.session_id,
                user_id=event.user_id,
                started_at=event.timestamp,
                last_activity=event.timestamp,
            )
            self._sessions[event.session_id] = session
        session.update(event)

    async def _flush_batch(self) -> None:
        batch = self._buffer[:]
        self._buffer.clear()
        BATCH_SIZE.observe(len(batch))
        for sink in self._sinks:
            try:
                await sink.write_batch(batch)
            except Exception:
                logger.exception("Sink %s failed to write batch", type(sink).__name__)

    async def _flush_sessions(self, *, force: bool = False) -> None:
        if force:
            sessions = list(self._sessions.values())
        else:
            now = datetime.now(timezone.utc)
            sessions = [
                s
                for s in self._sessions.values()
                if now - s.last_activity > timedelta(minutes=5)
            ]

        if sessions:
            for sink in self._sinks:
                await sink.flush_sessions(sessions)


# --- Entry Point ---


async def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    # Start Prometheus metrics server
    start_http_server(9090)

    consumer = AIOKafkaConsumer(
        "clickstream-events",
        bootstrap_servers="kafka:9092",
        group_id="clickstream-pipeline",
        auto_offset_reset="earliest",
        enable_auto_commit=False,
        max_poll_records=500,
    )

    pg_sink = PostgresSink("postgresql://app:secret@postgres:5432/analytics")
    await pg_sink.connect()

    pq_sink = ParquetSink(Path("/data/lake/clickstream"))

    pipeline = Pipeline(consumer, [pg_sink, pq_sink], batch_interval=5.0)

    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, lambda: asyncio.create_task(pipeline.stop()))

    await pipeline.start()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)
