-- Migration 001: Initial schema

CREATE TABLE IF NOT EXISTS captured_content (
    id              TEXT PRIMARY KEY,
    content_type    TEXT NOT NULL,
    raw_text        TEXT,
    image_path      TEXT,
    thumbnail_path  TEXT,
    source_app      TEXT NOT NULL,
    source_bundle_id TEXT,
    source_url      TEXT,
    captured_at     TEXT NOT NULL,
    content_hash    TEXT NOT NULL,
    byte_size       INTEGER NOT NULL DEFAULT 0,
    is_deleted      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_content_captured_at ON captured_content(captured_at);
CREATE INDEX IF NOT EXISTS idx_content_type ON captured_content(content_type);
CREATE INDEX IF NOT EXISTS idx_content_source ON captured_content(source_app);
CREATE INDEX IF NOT EXISTS idx_content_hash ON captured_content(content_hash);

CREATE TABLE IF NOT EXISTS weekly_reports (
    id              TEXT PRIMARY KEY,
    week_start      TEXT NOT NULL,
    week_end        TEXT NOT NULL,
    summary_text    TEXT NOT NULL,
    report_json     TEXT NOT NULL,
    content_count   INTEGER NOT NULL,
    model_used      TEXT NOT NULL,
    tokens_used     INTEGER,
    generated_at    TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_report_week ON weekly_reports(week_start);

CREATE TABLE IF NOT EXISTS report_sections (
    id              TEXT PRIMARY KEY,
    report_id       TEXT NOT NULL REFERENCES weekly_reports(id) ON DELETE CASCADE,
    section_type    TEXT NOT NULL,
    title           TEXT NOT NULL,
    body            TEXT NOT NULL,
    relevance_score REAL,
    sort_order      INTEGER NOT NULL,
    content_ids     TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_sections_report ON report_sections(report_id);

CREATE TABLE IF NOT EXISTS user_feedback (
    id              TEXT PRIMARY KEY,
    content_id      TEXT REFERENCES captured_content(id) ON DELETE SET NULL,
    section_id      TEXT REFERENCES report_sections(id) ON DELETE SET NULL,
    feedback_type   TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_feedback_content ON user_feedback(content_id);
CREATE INDEX IF NOT EXISTS idx_feedback_type ON user_feedback(feedback_type);

CREATE TABLE IF NOT EXISTS user_preferences (
    id              TEXT PRIMARY KEY,
    topic           TEXT NOT NULL,
    weight          REAL NOT NULL DEFAULT 0.0,
    occurrence_count INTEGER NOT NULL DEFAULT 0,
    last_updated    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_preferences_topic ON user_preferences(topic);

CREATE TABLE IF NOT EXISTS app_settings (
    key             TEXT PRIMARY KEY,
    value           TEXT NOT NULL,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO app_settings (key, value) VALUES
    ('ai_provider', 'anthropic'),
    ('ai_model', 'claude-sonnet-4-6'),
    ('ai_api_key', ''),
    ('screenshot_dir', ''),
    ('report_day', 'sunday'),
    ('report_time', '20:00'),
    ('capture_enabled', 'true'),
    ('countdown_seconds', '5'),
    ('theme', 'system'),
    ('url_reading_enabled', 'true');
