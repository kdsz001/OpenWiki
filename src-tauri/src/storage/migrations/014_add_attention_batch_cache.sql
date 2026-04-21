CREATE TABLE IF NOT EXISTS attention_batch_cache (
    batch_key        TEXT PRIMARY KEY,
    content_ids_json TEXT NOT NULL,
    report_json      TEXT NOT NULL,
    item_count       INTEGER NOT NULL,
    model_used       TEXT NOT NULL,
    locale           TEXT NOT NULL DEFAULT 'zh-CN',
    generated_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_attention_batch_generated_at
    ON attention_batch_cache(generated_at DESC);
