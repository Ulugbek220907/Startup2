-- QiyiqDoʻppi schema, v1.
-- Money is BIGINT tiyin (1 soʻm = 100 tiyin). Status columns are TEXT with
-- CHECK constraints instead of Postgres enums: adding a value later is a
-- one-line constraint change rather than an enum migration.

-- ---------------------------------------------------------------- catalog

CREATE TABLE garments (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug             TEXT NOT NULL UNIQUE,
    kind             TEXT NOT NULL CHECK (kind IN ('tshirt', 'hoodie', 'sweatshirt')),
    name_uz          TEXT NOT NULL,
    name_ru          TEXT NOT NULL,
    name_en          TEXT NOT NULL,
    base_price_tiyin BIGINT NOT NULL CHECK (base_price_tiyin >= 0),
    active           BOOLEAN NOT NULL DEFAULT TRUE,
    sort             INT NOT NULL DEFAULT 0
);

CREATE TABLE garment_colors (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    garment_id UUID NOT NULL REFERENCES garments (id) ON DELETE CASCADE,
    slug       TEXT NOT NULL,
    name_uz    TEXT NOT NULL,
    name_ru    TEXT NOT NULL,
    name_en    TEXT NOT NULL,
    hex        TEXT NOT NULL CHECK (hex ~ '^#[0-9a-f]{6}$'),
    sort       INT NOT NULL DEFAULT 0,
    UNIQUE (garment_id, slug)
);

CREATE TABLE garment_sizes (
    garment_id UUID NOT NULL REFERENCES garments (id) ON DELETE CASCADE,
    size       TEXT NOT NULL,
    sort       INT NOT NULL DEFAULT 0,
    PRIMARY KEY (garment_id, size)
);

-- Physical print area per garment side, plus its position on the garment
-- artwork as fractions (0..1) of the artwork's width and height.
CREATE TABLE print_areas (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    garment_id UUID NOT NULL REFERENCES garments (id) ON DELETE CASCADE,
    side       TEXT NOT NULL CHECK (side IN ('front', 'back')),
    width_cm   REAL NOT NULL CHECK (width_cm > 0 AND width_cm <= 100),
    height_cm  REAL NOT NULL CHECK (height_cm > 0 AND height_cm <= 100),
    mock_x     REAL NOT NULL CHECK (mock_x >= 0 AND mock_x <= 1),
    mock_y     REAL NOT NULL CHECK (mock_y >= 0 AND mock_y <= 1),
    mock_w     REAL NOT NULL CHECK (mock_w > 0 AND mock_w <= 1),
    mock_h     REAL NOT NULL CHECK (mock_h > 0 AND mock_h <= 1),
    UNIQUE (garment_id, side)
);

-- ---------------------------------------------------------------- pricing

CREATE TABLE price_tiers (
    max_area_cm2 INT PRIMARY KEY CHECK (max_area_cm2 > 0),
    price_tiyin  BIGINT NOT NULL CHECK (price_tiyin >= 0)
);

CREATE TABLE quantity_discounts (
    min_qty    INT PRIMARY KEY CHECK (min_qty > 0),
    percent_bp INT NOT NULL CHECK (percent_bp BETWEEN 0 AND 10000)
);

CREATE TABLE regions (
    slug               TEXT PRIMARY KEY,
    name_uz            TEXT NOT NULL,
    name_ru            TEXT NOT NULL,
    name_en            TEXT NOT NULL,
    delivery_fee_tiyin BIGINT NOT NULL CHECK (delivery_fee_tiyin >= 0),
    active             BOOLEAN NOT NULL DEFAULT TRUE,
    sort               INT NOT NULL DEFAULT 0
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value JSONB NOT NULL
);

-- -------------------------------------------------------------- customers

CREATE TABLE customers (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    phone            TEXT UNIQUE CHECK (phone ~ '^\+998[0-9]{9}$'),
    telegram_id      BIGINT UNIQUE,
    telegram_chat_id BIGINT,
    name             TEXT,
    lang             TEXT NOT NULL DEFAULT 'uz' CHECK (lang IN ('uz', 'ru', 'en')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (phone IS NOT NULL OR telegram_id IS NOT NULL)
);

-- ------------------------------------------------------ assets and designs

CREATE TABLE assets (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sha256      TEXT NOT NULL,
    mime        TEXT NOT NULL,
    width       INT NOT NULL,
    height      INT NOT NULL,
    bytes       BIGINT NOT NULL,
    storage_key TEXT NOT NULL,
    preview_key TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX assets_created_at ON assets (created_at);

CREATE TABLE designs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug              TEXT NOT NULL UNIQUE,
    edit_token_hash   TEXT NOT NULL,
    schema_version    INT NOT NULL,
    doc               JSONB NOT NULL,
    garment_slug      TEXT NOT NULL,
    moderation_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (moderation_status IN ('pending', 'clean', 'flagged', 'approved', 'rejected')),
    moderation_reason TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX designs_moderation ON designs (moderation_status) WHERE moderation_status = 'flagged';
CREATE INDEX designs_updated_at ON designs (updated_at);

CREATE TABLE design_reports (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    design_id   UUID NOT NULL REFERENCES designs (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('report', 'trademark')),
    reason      TEXT NOT NULL,
    contact     TEXT,
    status      TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'dismissed', 'removed')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    resolved_by TEXT
);
CREATE INDEX design_reports_open ON design_reports (created_at) WHERE status = 'open';

-- ----------------------------------------------------------------- orders

CREATE TABLE orders (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code               TEXT NOT NULL UNIQUE,
    customer_id        UUID REFERENCES customers (id) ON DELETE SET NULL,
    -- Nullable so a privacy deletion can anonymise finished orders while
    -- keeping the sales record.
    phone              TEXT,
    customer_name      TEXT,
    status             TEXT NOT NULL DEFAULT 'new'
        CHECK (status IN ('new', 'confirmed', 'in_production', 'ready', 'delivered', 'cancelled')),
    delivery_method    TEXT NOT NULL CHECK (delivery_method IN ('delivery', 'pickup')),
    region_slug        TEXT,
    address            TEXT,
    notes              TEXT,
    lang               TEXT NOT NULL CHECK (lang IN ('uz', 'ru', 'en')),
    payment_method     TEXT NOT NULL DEFAULT 'cash_on_delivery',
    -- The price is frozen at order time; later price changes do not alter it.
    subtotal_tiyin     BIGINT NOT NULL CHECK (subtotal_tiyin >= 0),
    discount_tiyin     BIGINT NOT NULL CHECK (discount_tiyin >= 0),
    delivery_fee_tiyin BIGINT NOT NULL CHECK (delivery_fee_tiyin >= 0),
    total_tiyin        BIGINT NOT NULL CHECK (total_tiyin >= 0),
    quote              JSONB NOT NULL,
    files_status       TEXT NOT NULL DEFAULT 'pending' CHECK (files_status IN ('pending', 'ready', 'failed')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (total_tiyin = subtotal_tiyin - discount_tiyin + delivery_fee_tiyin)
);
CREATE INDEX orders_created_at ON orders (created_at DESC);
CREATE INDEX orders_status ON orders (status, created_at DESC);
CREATE INDEX orders_phone ON orders (phone);

-- v1 orders have exactly one item; the table allows several for a v2 cart.
CREATE TABLE order_items (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id         UUID NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    design_id        UUID NOT NULL REFERENCES designs (id),
    -- Frozen copy of the design at order time.
    design_doc       JSONB NOT NULL,
    garment_slug     TEXT NOT NULL,
    garment_kind     TEXT NOT NULL,
    garment_name     TEXT NOT NULL,
    color_slug       TEXT NOT NULL,
    color_name       TEXT NOT NULL,
    color_hex        TEXT NOT NULL,
    size             TEXT NOT NULL,
    quantity         INT NOT NULL CHECK (quantity > 0),
    finish           TEXT NOT NULL DEFAULT 'standard' CHECK (finish IN ('standard', 'foil_gold', 'foil_silver')),
    unit_price_tiyin BIGINT NOT NULL CHECK (unit_price_tiyin >= 0),
    -- Print areas as they were at order time (JSON array), so re-rendering
    -- an old order is not affected by later print-area edits.
    areas            JSONB NOT NULL
);
CREATE INDEX order_items_order ON order_items (order_id);
CREATE INDEX order_items_design ON order_items (design_id);

CREATE TABLE order_status_log (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id    UUID NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    from_status TEXT,
    to_status   TEXT NOT NULL,
    actor       TEXT NOT NULL,
    note        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX order_status_log_order ON order_status_log (order_id, created_at);

CREATE TABLE order_files (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id    UUID NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('mockup_front', 'mockup_back', 'print_front', 'print_back', 'sheet')),
    storage_key TEXT NOT NULL,
    mime        TEXT NOT NULL,
    bytes       BIGINT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (order_id, kind)
);

-- Not used in v1 (cash on delivery). Present so adding Click/Payme in v2
-- needs no change to existing tables.
CREATE TABLE payments (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id     UUID NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    provider     TEXT NOT NULL,
    amount_tiyin BIGINT NOT NULL CHECK (amount_tiyin >= 0),
    status       TEXT NOT NULL CHECK (status IN ('pending', 'paid', 'failed', 'refunded')),
    external_id  TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, external_id)
);

-- ----------------------------------------------------------------- try-on

CREATE TABLE tryon_jobs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    device_id   TEXT NOT NULL,
    photo_key   TEXT,
    garment_key TEXT,
    result_key  TEXT,
    result_mime TEXT,
    provider    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'failed', 'deleted')),
    error       TEXT,
    cost_tiyin  BIGINT NOT NULL DEFAULT 0,
    -- sha256(photo) + sha256(garment): identical requests reuse a result.
    cache_key   TEXT NOT NULL,
    consent_at  TIMESTAMPTZ NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);
CREATE INDEX tryon_jobs_expires ON tryon_jobs (expires_at) WHERE status <> 'deleted';
CREATE INDEX tryon_jobs_cache ON tryon_jobs (cache_key) WHERE status = 'done';
CREATE INDEX tryon_jobs_created ON tryon_jobs (created_at);

-- ------------------------------------------------------------- operations

-- Background job queue. Workers claim rows with FOR UPDATE SKIP LOCKED.
CREATE TABLE jobs (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    public_id    UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    kind         TEXT NOT NULL,
    payload      JSONB NOT NULL,
    status       TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'failed')),
    run_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts     INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 5,
    locked_until TIMESTAMPTZ,
    last_error   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at  TIMESTAMPTZ
);
CREATE INDEX jobs_ready ON jobs (run_at) WHERE status = 'queued';
CREATE INDEX jobs_running ON jobs (locked_until) WHERE status = 'running';

CREATE TABLE admins (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    admin_id   UUID NOT NULL REFERENCES admins (id) ON DELETE CASCADE,
    -- SHA-256 of the cookie value; a database leak does not leak sessions.
    token_hash TEXT NOT NULL UNIQUE,
    csrf_token TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sessions_expires ON sessions (expires_at);

CREATE TABLE audit_log (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor      TEXT NOT NULL,
    action     TEXT NOT NULL,
    entity     TEXT NOT NULL,
    entity_id  TEXT,
    detail     JSONB NOT NULL DEFAULT '{}'::jsonb,
    ip         TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_log_created ON audit_log (created_at DESC);

CREATE TABLE blocklist_terms (
    term TEXT PRIMARY KEY
);

-- Fixed-window rate limiting counters (per IP, per phone, per device).
CREATE TABLE rate_limits (
    key          TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    count        INT NOT NULL DEFAULT 1,
    PRIMARY KEY (key, window_start)
);
CREATE INDEX rate_limits_window ON rate_limits (window_start);
