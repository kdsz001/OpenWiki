-- Migration 015: Rebuild wiki_pages_fts with CJK character segmentation.
--
-- Problem: the unicode61 tokenizer treats every continuous CJK sequence
-- as a SINGLE token. So "整理设计风格" indexes as one mega-token, and a
-- search for "设计" never matches because token-equality fails. This
-- silently broke recall for any Chinese phrase shorter than the full
-- run of ideographs in a document.
--
-- Fix: pre-process every searchable column through cjk_seg() (registered
-- as a SQLite scalar function in database.rs), which inserts a space
-- between adjacent CJK ideographs. Now "整理设计风格" is stored as
-- "整 理 设 计 风 格" — six independent tokens. The query side mirrors
-- this transform, so a search for "设计" becomes the phrase "设 计",
-- matching adjacent tokens correctly.

DROP TRIGGER IF EXISTS wiki_pages_fts_insert;
DROP TRIGGER IF EXISTS wiki_pages_fts_delete;
DROP TRIGGER IF EXISTS wiki_pages_fts_update;
DROP TABLE IF EXISTS wiki_pages_fts;

CREATE VIRTUAL TABLE wiki_pages_fts USING fts5(
    page_id UNINDEXED,
    title,
    summary,
    body,
    tags,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Backfill with CJK-segmented content
INSERT INTO wiki_pages_fts (page_id, title, summary, body, tags)
SELECT
    id,
    cjk_seg(title),
    cjk_seg(COALESCE(summary, '')),
    cjk_seg(body_markdown),
    cjk_seg(COALESCE(tags, ''))
FROM wiki_pages;

-- Re-create triggers, this time piping through cjk_seg()
CREATE TRIGGER wiki_pages_fts_insert
AFTER INSERT ON wiki_pages
BEGIN
    INSERT INTO wiki_pages_fts (page_id, title, summary, body, tags)
    VALUES (
        new.id,
        cjk_seg(new.title),
        cjk_seg(COALESCE(new.summary, '')),
        cjk_seg(new.body_markdown),
        cjk_seg(COALESCE(new.tags, ''))
    );
END;

CREATE TRIGGER wiki_pages_fts_delete
AFTER DELETE ON wiki_pages
BEGIN
    DELETE FROM wiki_pages_fts WHERE page_id = old.id;
END;

CREATE TRIGGER wiki_pages_fts_update
AFTER UPDATE ON wiki_pages
BEGIN
    DELETE FROM wiki_pages_fts WHERE page_id = old.id;
    INSERT INTO wiki_pages_fts (page_id, title, summary, body, tags)
    VALUES (
        new.id,
        cjk_seg(new.title),
        cjk_seg(COALESCE(new.summary, '')),
        cjk_seg(new.body_markdown),
        cjk_seg(COALESCE(new.tags, ''))
    );
END;
