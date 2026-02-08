# Architecture Decision Record: Event-Driven Order Pipeline

**Status**: Accepted
**Date**: 2024-12-01
**Authors**: Platform Team

## Context

Our monolithic order processing system handles ~50k orders/day with synchronous HTTP calls between services. During Black Friday 2023, cascading timeouts caused a 47-minute outage affecting $2.3M in lost revenue. The system cannot scale horizontally because:

1. **Tight coupling**: The order service directly calls payment, inventory, and notification services
2. **No backpressure**: A slow downstream service blocks the entire pipeline
3. **No retry semantics**: Failed payments require manual intervention
4. **Limited observability**: No distributed tracing across service boundaries

## Decision

We will migrate to an **event-driven architecture** using Apache Kafka as the message backbone, with the following topology:

```
┌─────────────┐    ┌───────┐    ┌──────────────────┐
│  API Gateway │───▶│ Kafka │───▶│  Order Service   │
└─────────────┘    │       │    └──────────────────┘
                   │       │            │
                   │       │    ┌───────▼──────────┐
                   │       │◀───│ Payment Service  │
                   │       │    └──────────────────┘
                   │       │            │
                   │       │    ┌───────▼──────────┐
                   │       │◀───│ Inventory Service│
                   │       │    └──────────────────┘
                   │       │            │
                   │       │    ┌───────▼──────────┐
                   │       │    │ Notification Svc │
                   └───────┘    └──────────────────┘
```

### Key Design Choices

| Decision | Choice | Alternatives Considered |
|----------|--------|------------------------|
| Message broker | Kafka | RabbitMQ, AWS SQS, NATS |
| Serialization | Protobuf + Schema Registry | JSON, Avro |
| Delivery guarantee | Exactly-once (idempotent consumers) | At-least-once |
| Partition strategy | By `customer_id` | By `order_id`, random |
| Consumer groups | One per service | Shared consumers |

### Event Schema

Orders flow through these states via events:

- [x] `OrderCreated` - Initial order placement
- [x] `PaymentRequested` - Sent to payment service
- [x] `PaymentConfirmed` / `PaymentFailed` - Payment result
- [x] `InventoryReserved` / `InventoryInsufficient` - Stock check
- [x] `OrderConfirmed` - All checks passed
- [ ] `OrderShipped` - Fulfillment complete (Phase 2)
- [ ] `OrderDelivered` - Delivery confirmed (Phase 2)

### Retry & Dead Letter Policy

```yaml
retry:
  max_attempts: 5
  backoff: exponential
  base_delay: 1s
  max_delay: 60s
  dead_letter_topic: "orders.dlq"
  alert_on_dlq: true
```

## Consequences

### Positive

- **Decoupled services**: Each service can be deployed, scaled, and fail independently
- **Natural backpressure**: Kafka consumer lag absorbs traffic spikes without cascading failures
- **Audit trail**: Kafka log provides a complete, ordered history of all state transitions
- **Replay capability**: Can replay events from any offset to rebuild state or debug issues
- **Horizontal scaling**: Add consumer instances to scale throughput linearly

### Negative

- **Eventual consistency**: Orders are no longer confirmed synchronously; UI must handle pending states
- **Operational complexity**: Kafka cluster requires dedicated ops expertise (ZooKeeper/KRaft, partition rebalancing, ISR monitoring)
- **Debugging difficulty**: Distributed traces across async events are harder to follow than synchronous call stacks
- **Schema evolution**: Breaking changes to event schemas require careful versioning and migration

### Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Kafka cluster failure | Low | Critical | Multi-AZ deployment, 3x replication, automated failover |
| Consumer lag during spikes | Medium | High | Auto-scaling consumer groups, lag alerting at 10k threshold |
| Poison pill messages | Medium | Medium | Schema validation, DLQ with alerting, circuit breaker per topic |
| Data loss on unclean shutdown | Low | Critical | `acks=all`, `min.insync.replicas=2`, idempotent producer |

## Performance Targets

> **Note**: All latency targets are P99 measured at the API gateway.

- Order placement to confirmation: **< 5 seconds** (currently 12s synchronous)
- Payment processing: **< 3 seconds** (P99, excluding bank-side latency)
- Throughput: **> 500 orders/sec** sustained (10x current peak)
- Consumer lag: **< 1000 messages** during normal operation
- Recovery time: **< 30 seconds** after consumer restart

## References

1. [Designing Event-Driven Systems](https://www.confluent.io/designing-event-driven-systems/) - Ben Stopford
2. [Kafka: The Definitive Guide](https://www.oreilly.com/library/view/kafka-the-definitive/9781492043072/) - Shapira et al.
3. Internal post-mortem: `INC-2024-BF-001` (Black Friday 2023 outage)
4. Prototype benchmark results: `docs/benchmarks/kafka-pilot-2024q3.pdf`

---

*This ADR supersedes [ADR-017: Synchronous Order Pipeline](adr-017.md) from 2022-03.*
