CREATE TABLE IF NOT EXISTS content_concept_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT NOT NULL,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    importance REAL NOT NULL DEFAULT 0.0,
    temporality TEXT NOT NULL DEFAULT 'transient',
    rationale TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(content_id, normalized_name),
    FOREIGN KEY (content_id) REFERENCES captured_content(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_content_concept_candidates_content_id
    ON content_concept_candidates(content_id);

CREATE INDEX IF NOT EXISTS idx_content_concept_candidates_normalized_name
    ON content_concept_candidates(normalized_name);
