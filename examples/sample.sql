-- E-commerce analytics schema and materialized views
-- Optimized for time-series queries on order and clickstream data

BEGIN;

-- Extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";

-- Enum types
CREATE TYPE order_status AS ENUM (
    'pending', 'confirmed', 'processing', 'shipped',
    'delivered', 'cancelled', 'refunded'
);

CREATE TYPE payment_method AS ENUM (
    'credit_card', 'debit_card', 'paypal',
    'apple_pay', 'google_pay', 'bank_transfer'
);

-- Core tables

CREATE TABLE customers (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email           TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    phone           TEXT,
    tier            TEXT NOT NULL DEFAULT 'standard' CHECK (tier IN ('standard', 'premium', 'enterprise')),
    metadata        JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_customers_email_trgm ON customers USING gin (email gin_trgm_ops);
CREATE INDEX idx_customers_tier ON customers (tier) WHERE tier != 'standard';
CREATE INDEX idx_customers_created ON customers (created_at DESC);

CREATE TABLE products (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    sku             TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    description     TEXT,
    category        TEXT NOT NULL,
    subcategory     TEXT,
    price_cents     INTEGER NOT NULL CHECK (price_cents >= 0),
    cost_cents      INTEGER NOT NULL CHECK (cost_cents >= 0),
    weight_grams    INTEGER,
    is_active       BOOLEAN NOT NULL DEFAULT true,
    tags            TEXT[] DEFAULT '{}',
    attributes      JSONB DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_products_category ON products (category, subcategory) WHERE is_active;
CREATE INDEX idx_products_tags ON products USING gin (tags);
CREATE INDEX idx_products_attrs ON products USING gin (attributes jsonb_path_ops);

CREATE TABLE orders (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    customer_id     UUID NOT NULL REFERENCES customers(id),
    status          order_status NOT NULL DEFAULT 'pending',
    payment_method  payment_method,
    subtotal_cents  INTEGER NOT NULL CHECK (subtotal_cents >= 0),
    tax_cents       INTEGER NOT NULL DEFAULT 0,
    shipping_cents  INTEGER NOT NULL DEFAULT 0,
    discount_cents  INTEGER NOT NULL DEFAULT 0,
    total_cents     INTEGER NOT NULL GENERATED ALWAYS AS (
        subtotal_cents + tax_cents + shipping_cents - discount_cents
    ) STORED,
    currency        TEXT NOT NULL DEFAULT 'USD' CHECK (length(currency) = 3),
    shipping_address JSONB,
    coupon_code     TEXT,
    notes           TEXT,
    placed_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at    TIMESTAMPTZ,
    shipped_at      TIMESTAMPTZ,
    delivered_at    TIMESTAMPTZ,
    cancelled_at    TIMESTAMPTZ
);

CREATE INDEX idx_orders_customer ON orders (customer_id, placed_at DESC);
CREATE INDEX idx_orders_status ON orders (status) WHERE status NOT IN ('delivered', 'cancelled');
CREATE INDEX idx_orders_placed ON orders (placed_at DESC);

CREATE TABLE order_items (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    order_id        UUID NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id      UUID NOT NULL REFERENCES products(id),
    sku             TEXT NOT NULL,
    quantity        INTEGER NOT NULL CHECK (quantity > 0),
    unit_price_cents INTEGER NOT NULL CHECK (unit_price_cents >= 0),
    total_cents     INTEGER NOT NULL GENERATED ALWAYS AS (quantity * unit_price_cents) STORED
);

CREATE INDEX idx_order_items_order ON order_items (order_id);
CREATE INDEX idx_order_items_product ON order_items (product_id);

-- Inventory tracking
CREATE TABLE inventory (
    product_id      UUID NOT NULL REFERENCES products(id),
    warehouse       TEXT NOT NULL,
    quantity        INTEGER NOT NULL DEFAULT 0 CHECK (quantity >= 0),
    reserved        INTEGER NOT NULL DEFAULT 0 CHECK (reserved >= 0),
    reorder_point   INTEGER NOT NULL DEFAULT 10,
    last_restocked  TIMESTAMPTZ,
    PRIMARY KEY (product_id, warehouse),
    CHECK (reserved <= quantity)
);

-- Clickstream event log (partitioned by month)
CREATE TABLE events (
    id              BIGINT GENERATED ALWAYS AS IDENTITY,
    event_type      TEXT NOT NULL,
    session_id      TEXT NOT NULL,
    user_id         UUID REFERENCES customers(id),
    timestamp       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    page_url        TEXT,
    referrer        TEXT,
    properties      JSONB DEFAULT '{}',
    user_agent      TEXT
) PARTITION BY RANGE (timestamp);

CREATE TABLE events_2024_12 PARTITION OF events
    FOR VALUES FROM ('2024-12-01') TO ('2025-01-01');
CREATE TABLE events_2025_01 PARTITION OF events
    FOR VALUES FROM ('2025-01-01') TO ('2025-02-01');

CREATE INDEX idx_events_session ON events (session_id, timestamp);
CREATE INDEX idx_events_user ON events (user_id, timestamp DESC) WHERE user_id IS NOT NULL;
CREATE INDEX idx_events_type ON events (event_type, timestamp DESC);

-- Materialized views for dashboards

CREATE MATERIALIZED VIEW mv_daily_revenue AS
SELECT
    date_trunc('day', o.placed_at)::DATE AS day,
    COUNT(DISTINCT o.id) AS order_count,
    COUNT(DISTINCT o.customer_id) AS unique_customers,
    SUM(o.total_cents) AS revenue_cents,
    AVG(o.total_cents)::INTEGER AS avg_order_cents,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY o.total_cents)::INTEGER AS median_order_cents,
    SUM(CASE WHEN o.status = 'cancelled' THEN 1 ELSE 0 END) AS cancelled_count,
    SUM(CASE WHEN o.coupon_code IS NOT NULL THEN 1 ELSE 0 END) AS coupon_orders
FROM orders o
WHERE o.placed_at >= NOW() - INTERVAL '90 days'
GROUP BY 1
ORDER BY 1 DESC;

CREATE UNIQUE INDEX ON mv_daily_revenue (day);

CREATE MATERIALIZED VIEW mv_product_performance AS
SELECT
    p.id AS product_id,
    p.sku,
    p.name,
    p.category,
    COUNT(DISTINCT oi.order_id) AS times_ordered,
    SUM(oi.quantity) AS units_sold,
    SUM(oi.total_cents) AS revenue_cents,
    SUM(oi.quantity * p.cost_cents) AS cost_cents,
    SUM(oi.total_cents) - SUM(oi.quantity * p.cost_cents) AS profit_cents,
    ROUND(
        (SUM(oi.total_cents) - SUM(oi.quantity * p.cost_cents))::NUMERIC
        / NULLIF(SUM(oi.total_cents), 0) * 100, 1
    ) AS margin_pct
FROM products p
JOIN order_items oi ON oi.product_id = p.id
JOIN orders o ON o.id = oi.order_id AND o.status NOT IN ('cancelled', 'refunded')
WHERE o.placed_at >= NOW() - INTERVAL '30 days'
GROUP BY p.id, p.sku, p.name, p.category
ORDER BY revenue_cents DESC;

CREATE UNIQUE INDEX ON mv_product_performance (product_id);

-- Funnel analysis function
CREATE OR REPLACE FUNCTION analyze_conversion_funnel(
    p_start_date TIMESTAMPTZ,
    p_end_date TIMESTAMPTZ,
    p_funnel_steps TEXT[] DEFAULT ARRAY['page_view', 'product_view', 'add_to_cart', 'checkout_start', 'purchase_complete']
) RETURNS TABLE (
    step_name TEXT,
    step_order INTEGER,
    unique_sessions BIGINT,
    conversion_rate NUMERIC
) AS $$
DECLARE
    total_sessions BIGINT;
BEGIN
    SELECT COUNT(DISTINCT session_id) INTO total_sessions
    FROM events
    WHERE event_type = p_funnel_steps[1]
      AND timestamp BETWEEN p_start_date AND p_end_date;

    RETURN QUERY
    SELECT
        step,
        idx,
        cnt,
        CASE WHEN total_sessions > 0
            THEN ROUND(cnt::NUMERIC / total_sessions * 100, 2)
            ELSE 0
        END
    FROM (
        SELECT
            unnest AS step,
            row_number() OVER ()::INTEGER AS idx,
            COUNT(DISTINCT e.session_id) AS cnt
        FROM unnest(p_funnel_steps) WITH ORDINALITY
        JOIN events e ON e.event_type = unnest
            AND e.timestamp BETWEEN p_start_date AND p_end_date
        GROUP BY unnest, ordinality
        ORDER BY ordinality
    ) sub;
END;
$$ LANGUAGE plpgsql STABLE;

-- Refresh materialized views (called by cron)
CREATE OR REPLACE PROCEDURE refresh_analytics_views()
LANGUAGE plpgsql
AS $$
BEGIN
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_daily_revenue;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_product_performance;
    RAISE NOTICE 'Analytics views refreshed at %', NOW();
END;
$$;

COMMIT;
