-- Migration 014: Add FTS5 full-text search index for wiki_pages
--
-- Why: Q&A retrieval and Compile Discovery currently dump the ENTIRE
-- wiki_pages summary list to the LLM on every call. At ~1000 pages this
-- approaches 40k tokens per request. FTS5 lets us pre-filter to ~30
-- candidates in SQL before the AI ever sees them.
--
-- Tokenizer: unicode61 with diacritic removal — works for English and
-- character-level CJK matching. Future upgrade path: jieba for proper
-- Chinese word segmentation.

-- Contentless FTS table — we store page_id (UNINDEXED) so we can join
-- back to wiki_pages, plus the searchable text columns.
CREATE VIRTUAL TABLE IF NOT EXISTS wiki_pages_fts USING fts5(
    page_id UNINDEXED,
    title,
    summary,
    body,
    tags,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Backfill from existing wiki_pages (only active, non-qa rows that the
-- AI retrieval cares about). page_type='qa' rows are excluded by the
-- query layer anyway, but we still index them — cheap, and lets future
-- features search them.
INSERT INTO wiki_pages_fts (page_id, title, summary, body, tags)
SELECT id, title, COALESCE(summary, ''), body_markdown, COALESCE(tags, '')
FROM wiki_pages
WHERE NOT EXISTS (
    SELECT 1 FROM wiki_pages_fts WHERE wiki_pages_fts.page_id = wiki_pages.id
);

-- Keep FTS in sync with main table via triggers. AFTER triggers fire
-- only on successful row writes, so partial-failure scenarios cannot
-- desync the index.
CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_insert
AFTER INSERT ON wiki_pages
BEGIN
    INSERT INTO wiki_pages_fts (page_id, title, summary, body, tags)
    VALUES (new.id, new.title, COALESCE(new.summary, ''), new.body_markdown, COALESCE(new.tags, ''));
END;

CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_delete
AFTER DELETE ON wiki_pages
BEGIN
    DELETE FROM wiki_pages_fts WHERE page_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS wiki_pages_fts_update
AFTER UPDATE ON wiki_pages
BEGIN
    DELETE FROM wiki_pages_fts WHERE page_id = old.id;
    INSERT INTO wiki_pages_fts (page_id, title, summary, body, tags)
    VALUES (new.id, new.title, COALESCE(new.summary, ''), new.body_markdown, COALESCE(new.tags, ''));
END;
