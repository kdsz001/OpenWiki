/// Wiki knowledge base compilation engine.
///
/// Core operations:
/// - assess: evaluate if content has knowledge value
/// - compile: incrementally build wiki pages from content
/// - query: answer questions based on compiled wiki
/// - lint: health-check the wiki
use crate::storage::database::Database;
use crate::storage::models::{CapturedContent, ConceptCandidate, WikiPage};
use crate::storage::repository::Repository;
use crate::workspace;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use super::wiki_prompts;

const IMPORTED_CONCEPT_POLICY_VERSION: &str = "candidate-concepts-v3";
const LOCAL_WIKI_SOURCE_PREFIX: &str = "local-wiki:";
const THEME_EDGE_RELATION: &str = "belongs_to_theme";
const DEFAULT_MIN_PROMOTED_CONCEPT_IMPORTANCE: f64 = 0.65;
const DEFAULT_MIN_PROMOTED_CONCEPT_SOURCES: i64 = 2;
const DEFAULT_MIN_PROMOTED_CONCEPT_DAYS: i64 = 2;
const MAX_CANDIDATE_CONCEPTS: usize = 3;

#[derive(Clone, Copy)]
struct ConceptPromotionSettings {
    min_importance: f64,
    min_sources: i64,
    min_days: i64,
}

/// Compute a hash of the content's current state for change detection.
pub fn compute_content_hash(content: &CapturedContent) -> String {
    compute_content_hash_with_policy(content, None)
}

fn load_concept_promotion_settings(repo: &Repository) -> ConceptPromotionSettings {
    let min_importance = repo
        .get_setting("wiki_concept_min_importance")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.clamp(0.0, 1.0))
        .filter(|value| *value > 0.0)
        .unwrap_or(DEFAULT_MIN_PROMOTED_CONCEPT_IMPORTANCE);
    let min_sources = repo
        .get_setting("wiki_concept_min_sources")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.max(1))
        .unwrap_or(DEFAULT_MIN_PROMOTED_CONCEPT_SOURCES);
    let min_days = repo
        .get_setting("wiki_concept_min_days")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.max(1))
        .unwrap_or(DEFAULT_MIN_PROMOTED_CONCEPT_DAYS);

    ConceptPromotionSettings {
        min_importance,
        min_sources,
        min_days,
    }
}

fn compute_content_hash_with_policy(
    content: &CapturedContent,
    concept_settings: Option<ConceptPromotionSettings>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    if Path::new(&content.source_app)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt"))
        .unwrap_or(false)
    {
        IMPORTED_CONCEPT_POLICY_VERSION.hash(&mut hasher);
        let settings = concept_settings.unwrap_or(ConceptPromotionSettings {
            min_importance: DEFAULT_MIN_PROMOTED_CONCEPT_IMPORTANCE,
            min_sources: DEFAULT_MIN_PROMOTED_CONCEPT_SOURCES,
            min_days: DEFAULT_MIN_PROMOTED_CONCEPT_DAYS,
        });
        format!(
            "{:.2}:{}:{}",
            settings.min_importance, settings.min_sources, settings.min_days
        )
        .hash(&mut hasher);
    }
    // Prefer clean_content for hash computation — ensures re-compilation when cleaned
    let text = content
        .clean_content
        .as_deref()
        .or(content.raw_text.as_deref())
        .unwrap_or("");
    text.hash(&mut hasher);
    content.summary.as_deref().unwrap_or("").hash(&mut hasher);
    content.tags.as_deref().unwrap_or("").hash(&mut hasher);
    content.digest.as_deref().unwrap_or("").hash(&mut hasher);
    content.user_note.as_deref().unwrap_or("").hash(&mut hasher);
    content
        .source_url
        .as_deref()
        .unwrap_or("")
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn compute_content_hash_for_repo(repo: &Repository, content: &CapturedContent) -> String {
    compute_content_hash_with_policy(content, Some(load_concept_promotion_settings(repo)))
}

pub fn compute_content_hash_with_db(db: Arc<Database>, content: &CapturedContent) -> String {
    let repo = Repository::new(db);
    compute_content_hash_for_repo(&repo, content)
}

/// Generate a URL-safe slug from a title.
fn slugify(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_lowercase().next().unwrap_or(c)
            } else if c == ' ' {
                '-'
            } else {
                // Keep CJK characters as-is
                if c as u32 > 0x2E80 {
                    c
                } else {
                    '-'
                }
            }
        })
        .collect();
    // Collapse multiple dashes
    let mut result = String::new();
    let mut last_was_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_was_dash {
                result.push(c);
            }
            last_was_dash = true;
        } else {
            result.push(c);
            last_was_dash = false;
        }
    }
    result.trim_matches('-').to_string()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn parse_tag_terms(raw_tags: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for tag in raw_tags
        .split([',', '\n', ';', '|'])
        .map(str::trim)
        .filter(|term| !term.is_empty())
    {
        let tag = tag.to_string();
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

fn normalize_content_tags_for_page(tags: Option<&str>) -> Option<String> {
    let parsed = parse_tag_terms(tags.unwrap_or(""));
    if parsed.is_empty() {
        None
    } else {
        serde_json::to_string(&parsed).ok()
    }
}

fn parse_page_tags(raw_tags: Option<&str>) -> Vec<String> {
    let Some(raw_tags) = raw_tags else {
        return Vec::new();
    };
    if let Ok(tags) = serde_json::from_str::<Vec<String>>(raw_tags) {
        return parse_tag_terms(&tags.join(","));
    }
    parse_tag_terms(raw_tags)
}

fn merge_unique_tags(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut merged = existing.to_vec();
    for tag in incoming {
        if !merged.contains(tag) {
            merged.push(tag.clone());
        }
    }
    merged
}

#[derive(Default)]
struct ExtractedMetadata {
    summary: String,
    tags: Vec<String>,
    digest: String,
    candidate_concepts: Vec<ConceptCandidate>,
}

fn normalize_candidate_temporality(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "recurring" | "recurrent" | "persistent" | "long_term" | "long-term" => {
            Some("recurring".to_string())
        }
        "transient" | "temporary" | "ephemeral" | "one_off" | "one-off" | "single_use"
        | "single-use" => Some("transient".to_string()),
        _ => None,
    }
}

fn normalize_candidate_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = slugify(trimmed);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn extract_structured_metadata(raw: &str) -> ExtractedMetadata {
    let trimmed = raw.trim();
    let cleaned = if trimmed.starts_with("```") {
        let without_prefix = if let Some(rest) = trimmed.strip_prefix("```json") {
            rest
        } else {
            &trimmed[3..]
        };
        without_prefix
            .strip_suffix("```")
            .unwrap_or(without_prefix)
            .trim()
    } else {
        trimmed
    };

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
        let summary = v
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let digest = v
            .get("digest")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let tags = v
            .get("tags")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let candidate_concepts = v
            .get("candidate_concepts")
            .and_then(|value| value.as_array())
            .map(|items| {
                let mut parsed = Vec::new();
                for item in items {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    let name = obj
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let Some(normalized_name) = normalize_candidate_name(&name) else {
                        continue;
                    };
                    let Some(temporality) = obj
                        .get("temporality")
                        .and_then(|value| value.as_str())
                        .and_then(normalize_candidate_temporality)
                    else {
                        continue;
                    };
                    let importance = obj
                        .get("importance")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0);
                    let rationale = obj
                        .get("rationale")
                        .and_then(|value| value.as_str())
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                    if parsed.iter().any(|candidate: &ConceptCandidate| {
                        candidate.normalized_name == normalized_name
                    }) {
                        continue;
                    }
                    parsed.push(ConceptCandidate {
                        name,
                        normalized_name,
                        importance,
                        temporality,
                        rationale,
                    });
                    if parsed.len() >= MAX_CANDIDATE_CONCEPTS {
                        break;
                    }
                }
                parsed
            })
            .unwrap_or_default();
        if !summary.is_empty() || !tags.is_empty() || !candidate_concepts.is_empty() {
            return ExtractedMetadata {
                summary,
                tags,
                digest,
                candidate_concepts,
            };
        }
    }

    ExtractedMetadata {
        summary: trimmed.to_string(),
        tags: Vec::new(),
        digest: String::new(),
        candidate_concepts: Vec::new(),
    }
}

fn build_metadata_prompt(text: &str, locale: &str) -> String {
    if crate::locale::is_english(locale) {
        format!(
            "Read the following content and return JSON with four fields:\n\
             1. \"tags\": 2-4 concrete retrieval tags. They can be specific and detailed.\n\
             2. \"summary\": Plain-English explanation of what this content is about (under 40 words).\n\
             3. \"digest\": Core takeaways from the content (80-120 words).\n\
             4. \"candidate_concepts\": 0-3 concept candidates. These are NOT just tags.\n\
                A candidate concept should be a reusable long-term theme, method, problem, strategy, or recurring topic that would still matter if this single note disappeared.\n\
                Do NOT elevate one-off diary fragments, exact trade numbers, temporary moods, short-lived anecdotes, or hyper-local details.\n\
                For each item return:\n\
                - \"name\": short concept name\n\
                - \"importance\": float 0..1\n\
                - \"temporality\": \"recurring\" or \"transient\"\n\
                - \"rationale\": one short sentence\n\
             Return JSON only.\n\n{}",
            text
        )
    } else {
        format!(
            "通读以下全文，返回JSON格式，包含四个字段：\n\
             1. \"tags\": 2-4个检索标签，可以具体、细节化，便于之后搜索。\n\
             2. \"summary\": 用大白话说这篇内容讲了什么（中文简体，不超过80字）。\n\
             3. \"digest\": 这篇内容的核心要点总结（中文简体，150-200字）。\n\
             4. \"candidate_concepts\": 0-3个“候选概念”。\n\
                候选概念不是普通标签，而是值得长期沉淀的主题、方法、问题、策略或反复出现的话题。\n\
                不要把一次性的日记片段、具体交易编号、短暂情绪、局部见闻、偶发细节直接升成概念。\n\
                每个候选概念返回：\n\
                - \"name\": 概念名\n\
                - \"importance\": 0到1之间的小数\n\
                - \"temporality\": \"recurring\" 或 \"transient\"\n\
                - \"rationale\": 一句很短的判断理由\n\
             只返回JSON。\n\n{}",
            text
        )
    }
}

async fn ensure_content_metadata(
    db: Arc<Database>,
    content: &CapturedContent,
) -> Result<CapturedContent, String> {
    let repo = Repository::new(db.clone());
    let summary_missing = content.summary.as_deref().unwrap_or("").trim().is_empty();
    let tags_missing = parse_tag_terms(content.tags.as_deref().unwrap_or("")).is_empty();
    let candidates_missing = repo
        .get_content_concept_candidates(&content.id)
        .map_err(|e| format!("Failed to load concept candidates: {}", e))?
        .is_empty();
    if !summary_missing && !tags_missing && !candidates_missing {
        return Ok(content.clone());
    }

    let text = content
        .clean_content
        .as_deref()
        .or(content.raw_text.as_deref())
        .unwrap_or("")
        .trim();
    if text.len() < 50 {
        return Ok(content.clone());
    }

    let locale = crate::locale::resolve_locale(&db);
    let excerpt = truncate_chars(text, 5000);
    let prompt = build_metadata_prompt(&excerpt, &locale);
    let raw = match call_ai(
        db.clone(),
        "You are an AI assistant that analyzes content and returns JSON.",
        &prompt,
        1024,
    )
    .await
    {
        Ok(raw) => raw,
        Err(err) => {
            log::warn!(
                "Metadata enrichment skipped for {} because AI call failed: {}",
                content.id,
                err
            );
            return Ok(content.clone());
        }
    };

    let metadata = extract_structured_metadata(&raw);
    if metadata.summary.trim().is_empty()
        && metadata.tags.is_empty()
        && metadata.candidate_concepts.is_empty()
    {
        return Ok(content.clone());
    }

    let final_summary = if metadata.summary.trim().is_empty() {
        content.summary.clone().unwrap_or_default()
    } else {
        metadata.summary
    };
    let final_tags = if metadata.tags.is_empty() {
        content.tags.clone().unwrap_or_default()
    } else {
        metadata.tags.join(",")
    };
    let final_digest = if metadata.digest.trim().is_empty() {
        content.digest.clone().unwrap_or_default()
    } else {
        metadata.digest
    };

    repo.update_summary_and_tags(&content.id, &final_summary, &final_tags, &final_digest)
        .map_err(|e| format!("Failed to save generated metadata: {}", e))?;
    repo.replace_content_concept_candidates(&content.id, &metadata.candidate_concepts)
        .map_err(|e| format!("Failed to save concept candidates: {}", e))?;
    let _ = workspace::sync_content_candidates_to_workspace(
        &repo,
        content,
        &metadata.candidate_concepts,
    );

    repo.get_content_by_id(&content.id)
        .map_err(|e| format!("Failed to reload content metadata: {}", e))?
        .ok_or_else(|| format!("Content {} disappeared after metadata update", content.id))
}

fn build_wiki_query_terms(
    content_text: &str,
    content_summary: &str,
    content_tags: &str,
    user_note: &str,
) -> Vec<String> {
    let mut terms = Vec::new();

    for tag in content_tags
        .split([',', '\n', ';', '|'])
        .map(str::trim)
        .filter(|term| !term.is_empty())
    {
        terms.push(tag.to_string());
    }

    for phrase in [content_summary, user_note]
        .into_iter()
        .flat_map(|text| text.split(['\n', '。', '.', '；', ';']))
        .map(str::trim)
        .filter(|term| term.chars().count() >= 3)
    {
        terms.push(truncate_chars(phrase, 40));
    }

    let compact_text = content_text
        .split_whitespace()
        .take(12)
        .collect::<Vec<_>>()
        .join(" ");
    if compact_text.chars().count() >= 6 {
        terms.push(truncate_chars(&compact_text, 60));
    } else if content_text.chars().count() >= 8 {
        terms.push(truncate_chars(content_text, 24));
    }

    let mut deduped = Vec::new();
    for term in terms {
        if !deduped.contains(&term) {
            deduped.push(term);
        }
        if deduped.len() >= 10 {
            break;
        }
    }
    deduped
}

fn collect_relevant_page_summaries(
    repo: &Repository,
    content_text: &str,
    content_summary: &str,
    content_tags: &str,
    user_note: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let terms = build_wiki_query_terms(content_text, content_summary, content_tags, user_note);
    let mut scores: HashMap<String, (usize, String, String)> = HashMap::new();

    for term in &terms {
        let matches = repo
            .search_wiki_pages(term, 8)
            .map_err(|e| format!("Failed to search wiki pages: {}", e))?;
        for page in matches {
            let entry = scores.entry(page.id.clone()).or_insert((
                0usize,
                page.title.clone(),
                page.summary.clone().unwrap_or_default(),
            ));
            entry.0 += 1;
            if entry.2.is_empty() {
                entry.2 = page.summary.unwrap_or_default();
            }
        }
    }

    let mut candidates: Vec<(usize, String, String, String)> = scores
        .into_iter()
        .map(|(id, (score, title, summary))| (score, id, title, summary))
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));

    let mut page_summaries = candidates
        .into_iter()
        .take(12)
        .map(|(_, id, title, summary)| (id, title, summary))
        .collect::<Vec<_>>();

    if page_summaries.is_empty() {
        page_summaries = repo
            .get_wiki_page_summaries()
            .map_err(|e| format!("Failed to get page index: {}", e))?;
        page_summaries.truncate(12);
    }

    Ok(page_summaries)
}

fn is_file_import_source(content: &CapturedContent) -> bool {
    Path::new(&content.source_app)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt"))
        .unwrap_or(false)
}

fn get_structured_page_ids_for_content(
    repo: &Repository,
    content_id: &str,
) -> Result<Vec<String>, String> {
    let mut page_ids = Vec::new();
    for source in repo
        .get_pages_for_content(content_id)
        .map_err(|e| e.to_string())?
    {
        if source.source_status != "active" {
            continue;
        }
        let Some(page) = repo
            .get_wiki_page_by_id(&source.page_id)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        if page.page_type != "source" && !page_ids.contains(&page.id) {
            page_ids.push(page.id);
        }
    }
    Ok(page_ids)
}

fn sync_source_derivation_edges(
    repo: &Repository,
    source_page_ids: &[String],
    structured_page_ids: &[String],
) -> Result<(), String> {
    for source_page_id in source_page_ids {
        repo.delete_edges_for_page_with_relation(source_page_id, "derived_from")
            .map_err(|e| format!("Failed to clear derived edges for source page: {}", e))?;
    }

    for structured_page_id in structured_page_ids {
        for source_page_id in source_page_ids {
            repo.save_wiki_edge(structured_page_id, source_page_id, "derived_from", 1.0)
                .map_err(|e| format!("Failed to save source derivation edge: {}", e))?;
        }
    }

    Ok(())
}

pub fn content_needs_structured_backfill(
    repo: &Repository,
    content: &CapturedContent,
) -> Result<bool, String> {
    if !is_file_import_source(content) {
        return Ok(false);
    }

    let summary_missing = content.summary.as_deref().unwrap_or("").trim().is_empty();
    let tags_missing = parse_tag_terms(content.tags.as_deref().unwrap_or("")).is_empty();
    let candidates_missing = repo
        .get_content_concept_candidates(&content.id)
        .map_err(|e| e.to_string())?
        .is_empty();
    let concept_settings = load_concept_promotion_settings(repo);
    if summary_missing || tags_missing || candidates_missing {
        return Ok(true);
    }

    let structured_page_ids = get_structured_page_ids_for_content(repo, &content.id)?;
    for structured_page_id in &structured_page_ids {
        let Some(page) = repo
            .get_wiki_page_by_id(structured_page_id)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        if page.page_type == "concept" {
            let (active_sources, _) = repo
                .count_active_sources(&page.id)
                .map_err(|e| e.to_string())?;
            let active_days = repo
                .count_active_source_days_for_page(&page.id)
                .map_err(|e| e.to_string())?;
            if active_sources < concept_settings.min_sources
                || active_days < concept_settings.min_days
            {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn infer_manual_page_title(content: &CapturedContent) -> String {
    if let Some(text) = content
        .clean_content
        .as_deref()
        .or(content.raw_text.as_deref())
    {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let heading = trimmed.trim_start_matches('#').trim();
            if !heading.is_empty() {
                return truncate_chars(heading, 60);
            }
            return truncate_chars(trimmed, 60);
        }
    }

    if !content.source_app.trim().is_empty() {
        let source_name = Path::new(&content.source_app)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(content.source_app.as_str());
        return truncate_chars(source_name, 60);
    }

    "Imported Source".to_string()
}

fn build_manual_source_body(content: &CapturedContent, page_type: &str) -> String {
    let content_text = content
        .clean_content
        .as_deref()
        .or(content.raw_text.as_deref())
        .unwrap_or("");
    let mut body = String::new();

    if page_type == "case" {
        body.push_str("## Case Context\n\n");
    } else {
        body.push_str("## Source\n\n");
    }
    body.push_str(&format!("- Captured at: {}\n", content.captured_at));
    body.push_str(&format!("- Source app: {}\n", content.source_app));
    if let Some(url) = &content.source_url {
        if !url.is_empty() {
            if let Some(local_path) = url.strip_prefix("local-raw:") {
                body.push_str(&format!("- Original file: {}\n", local_path));
            } else {
                body.push_str(&format!("- Original URL: {}\n", url));
            }
        }
    }
    body.push('\n');
    if page_type == "case" {
        body.push_str("## Case Notes\n\n");
    } else {
        body.push_str("## Original Content\n\n");
    }
    body.push_str(content_text);
    body.push('\n');

    body
}

fn ensure_unique_slug(repo: &Repository, slug: String, page_id: &str) -> Result<String, String> {
    if repo
        .get_wiki_page_by_slug(&slug)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        Ok(format!("{}-{}", slug, &page_id[..8]))
    } else {
        Ok(slug)
    }
}

fn create_manual_source_page(
    repo: &Repository,
    content: &CapturedContent,
    current_hash: &str,
) -> Result<Vec<String>, String> {
    let linked_pages = repo
        .get_pages_for_content(&content.id)
        .map_err(|e| e.to_string())?;
    let target_page_type = if is_file_import_source(content) {
        "case"
    } else {
        "source"
    };

    let title = infer_manual_page_title(content);
    let summary_source = content
        .summary
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            truncate_chars(
                content
                    .clean_content
                    .as_deref()
                    .or(content.raw_text.as_deref())
                    .unwrap_or(""),
                120,
            )
        });
    let page_tags = normalize_content_tags_for_page(content.tags.as_deref());
    let body_markdown = build_manual_source_body(content, target_page_type);
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut source_page_ids = Vec::new();

    for source in linked_pages {
        let Some(existing_page) = repo
            .get_wiki_page_by_id(&source.page_id)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        if existing_page.page_type != target_page_type
            && !(target_page_type == "case" && existing_page.page_type == "source")
        {
            continue;
        }

        let updated_page = WikiPage {
            id: existing_page.id.clone(),
            title: title.clone(),
            slug: existing_page.slug.clone(),
            page_type: target_page_type.to_string(),
            body_markdown: body_markdown.clone(),
            summary: Some(summary_source.clone()),
            tags: page_tags.clone(),
            status: "active".to_string(),
            confidence: existing_page.confidence,
            created_at: existing_page.created_at.clone(),
            updated_at: now.clone(),
            last_compiled_at: Some(now.clone()),
            source_message_id: existing_page.source_message_id.clone(),
        };
        repo.update_wiki_page(&updated_page)
            .map_err(|e| format!("Failed to update fallback source page: {}", e))?;
        repo.add_page_source(&existing_page.id, &content.id, current_hash)
            .map_err(|e| format!("Failed to refresh fallback source relation: {}", e))?;
        source_page_ids.push(existing_page.id);
    }

    if !source_page_ids.is_empty() {
        return Ok(source_page_ids);
    }

    let page_id = uuid::Uuid::new_v4().to_string();
    let slug = ensure_unique_slug(repo, slugify(&title), &page_id)?;

    let page = WikiPage {
        id: page_id.clone(),
        title,
        slug,
        page_type: target_page_type.to_string(),
        body_markdown,
        summary: Some(summary_source),
        tags: page_tags,
        status: "active".to_string(),
        confidence: 1.0,
        created_at: now.clone(),
        updated_at: now.clone(),
        last_compiled_at: Some(now),
        source_message_id: None,
    };

    repo.save_wiki_page(&page)
        .map_err(|e| format!("Failed to save fallback source page: {}", e))?;
    repo.add_page_source(&page_id, &content.id, current_hash)
        .map_err(|e| format!("Failed to save fallback source relation: {}", e))?;

    Ok(vec![page_id])
}

fn build_promoted_concept_summary(
    name: &str,
    rationale: Option<&str>,
    source_count: i64,
    day_count: i64,
) -> String {
    if let Some(rationale) = rationale.filter(|value| !value.trim().is_empty()) {
        format!(
            "{}（已在 {} 条来源、{} 天中反复出现）",
            rationale.trim(),
            source_count,
            day_count
        )
    } else {
        format!(
            "“{}”已在 {} 条来源、{} 天中反复出现，适合作为长期概念保留。",
            name, source_count, day_count
        )
    }
}

fn build_promoted_concept_body(
    name: &str,
    rationale: Option<&str>,
    avg_importance: f64,
    source_count: i64,
    day_count: i64,
    content: &CapturedContent,
) -> String {
    let digest = content
        .digest
        .as_deref()
        .filter(|digest| !digest.trim().is_empty())
        .or(content.summary.as_deref())
        .unwrap_or("这是一组持续重复出现、适合沉淀为知识节点的主题。");
    let rationale = rationale
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("该主题在多条内容中反复出现，且具有长期复用价值。");

    format!(
        "# {name}\n\n## 晋升依据\n\n- 复现来源：{source_count} 条\n- 分布天数：{day_count} 天\n- 平均重要度：{avg_importance:.2}\n- 判断理由：{rationale}\n\n## 主题说明\n\n{digest}\n\n## 最近线索\n\n- 最近来源：{source}\n- 最近记录时间：{captured_at}\n",
        name = name,
        source_count = source_count,
        day_count = day_count,
        avg_importance = avg_importance,
        rationale = rationale,
        digest = digest,
        source = content.source_app,
        captured_at = content.captured_at
    )
}

fn clear_existing_concept_sources_for_content(
    repo: &Repository,
    content_id: &str,
) -> Result<(), String> {
    for source in repo
        .get_pages_for_content(content_id)
        .map_err(|e| e.to_string())?
    {
        let Some(page) = repo
            .get_wiki_page_by_id(&source.page_id)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        if page.page_type == "concept" {
            repo.update_source_status(&page.id, content_id, "deleted")
                .map_err(|e| format!("Failed to clear stale concept source relation: {}", e))?;
        }
    }
    Ok(())
}

fn upsert_promoted_candidate_pages(
    repo: &Repository,
    content: &CapturedContent,
) -> Result<Vec<String>, String> {
    let concept_settings = load_concept_promotion_settings(repo);
    let incoming_tags = parse_tag_terms(content.tags.as_deref().unwrap_or(""));
    let candidates = repo
        .get_content_concept_candidates(&content.id)
        .map_err(|e| format!("Failed to load content concept candidates: {}", e))?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut page_ids = Vec::new();

    for candidate in &candidates {
        if candidate.temporality != "recurring"
            || candidate.importance < concept_settings.min_importance
        {
            continue;
        }

        let (supporting_content_ids, source_count, day_count, avg_importance, display_name) = repo
            .get_promotable_candidate_support(
                &candidate.normalized_name,
                concept_settings.min_importance,
            )
            .map_err(|e| format!("Failed to inspect candidate support: {}", e))?;

        if source_count < concept_settings.min_sources || day_count < concept_settings.min_days {
            continue;
        }

        let concept_name = display_name.unwrap_or_else(|| candidate.name.clone());
        let slug = candidate.normalized_name.clone();
        let existing_page = repo
            .get_wiki_page_by_slug(&slug)
            .map_err(|e| e.to_string())?
            .filter(|page| page.page_type == "concept");
        let merged_tags = merge_unique_tags(&incoming_tags, std::slice::from_ref(&concept_name));
        let page_summary = build_promoted_concept_summary(
            &concept_name,
            candidate.rationale.as_deref(),
            source_count,
            day_count,
        );
        let page_body = build_promoted_concept_body(
            &concept_name,
            candidate.rationale.as_deref(),
            avg_importance,
            source_count,
            day_count,
            content,
        );

        let page_id = match existing_page {
            Some(page) => {
                let updated_page = WikiPage {
                    id: page.id.clone(),
                    title: concept_name.clone(),
                    slug: page.slug.clone(),
                    page_type: page.page_type.clone(),
                    body_markdown: page_body.clone(),
                    summary: Some(page_summary.clone()),
                    tags: serde_json::to_string(&merged_tags).ok(),
                    status: "active".to_string(),
                    confidence: page.confidence.max(0.7),
                    created_at: page.created_at.clone(),
                    updated_at: now.clone(),
                    last_compiled_at: Some(now.clone()),
                    source_message_id: page.source_message_id.clone(),
                };
                repo.update_wiki_page(&updated_page)
                    .map_err(|e| format!("Failed to update promoted concept page: {}", e))?;
                page.id
            }
            None => {
                let page_id = uuid::Uuid::new_v4().to_string();
                let page = WikiPage {
                    id: page_id.clone(),
                    title: concept_name.clone(),
                    slug: ensure_unique_slug(repo, slug, &page_id)?,
                    page_type: "concept".to_string(),
                    body_markdown: page_body.clone(),
                    summary: Some(page_summary.clone()),
                    tags: serde_json::to_string(&merged_tags).ok(),
                    status: "active".to_string(),
                    confidence: 1.0,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    last_compiled_at: Some(now.clone()),
                    source_message_id: None,
                };
                repo.save_wiki_page(&page)
                    .map_err(|e| format!("Failed to save promoted concept page: {}", e))?;
                page_id
            }
        };

        for supporting_content_id in &supporting_content_ids {
            let Some(supporting_content) = repo
                .get_content_by_id(supporting_content_id)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            let supporting_hash = compute_content_hash_for_repo(repo, &supporting_content);
            repo.add_page_source(&page_id, supporting_content_id, &supporting_hash)
                .map_err(|e| format!("Failed to attach promoted concept source: {}", e))?;
        }

        if !page_ids.contains(&page_id) {
            page_ids.push(page_id);
        }
    }

    Ok(page_ids)
}

fn build_theme_match_system_prompt(locale: &str) -> String {
    if locale.starts_with("zh") {
        "你是个人知识库的信息架构师。请把新内容挂到最合适的既有主题上。\n\
         只从给定主题里选择，不要发明新主题。\n\
         选择标准：主题必须是长期稳定的上位领域，而不是一次性事件；优先选择能长期容纳这条内容的主题。\n\
         如果没有合适主题，返回空数组。\n\
         只返回 JSON：{\"theme_ids\": [\"...\"]}。最多选择 3 个 theme_ids。".to_string()
    } else {
        "You are an information architect for a personal knowledge base.\n\
         Attach the new content to the most suitable existing themes.\n\
         Only choose from the provided themes. Do not invent new themes.\n\
         Prefer durable parent domains that can continue to absorb similar notes over time.\n\
         If nothing fits, return an empty array.\n\
         Return JSON only: {\"theme_ids\": [\"...\"]}. Pick at most 3 theme_ids."
            .to_string()
    }
}

fn build_theme_match_user_message(
    content: &CapturedContent,
    theme_pages: &[WikiPage],
    candidates: &[ConceptCandidate],
    locale: &str,
) -> String {
    let summary = content.summary.as_deref().unwrap_or("");
    let digest = content.digest.as_deref().unwrap_or("");
    let tags = content.tags.as_deref().unwrap_or("");
    let note = content.user_note.as_deref().unwrap_or("");
    let source = &content.source_app;
    let candidate_text = if candidates.is_empty() {
        if locale.starts_with("zh") {
            "无".to_string()
        } else {
            "None".to_string()
        }
    } else {
        candidates
            .iter()
            .map(|candidate| {
                format!(
                    "- {} (importance={:.2}, temporality={})",
                    candidate.name, candidate.importance, candidate.temporality
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let themes_text = theme_pages
        .iter()
        .map(|page| {
            format!(
                "- id: {}\n  title: {}\n  summary: {}\n  tags: {}",
                page.id,
                page.title,
                page.summary.as_deref().unwrap_or(""),
                page.tags.as_deref().unwrap_or("[]")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    if locale.starts_with("zh") {
        format!(
            "内容来源：{source}\n\
             摘要：{summary}\n\
             digest：{digest}\n\
             tags：{tags}\n\
             用户备注：{note}\n\
             候选概念：\n{candidate_text}\n\n\
             可选主题：\n{themes_text}\n\n\
             请选择最适合承接这条内容的现有主题。"
        )
    } else {
        format!(
            "Source: {source}\n\
             Summary: {summary}\n\
             Digest: {digest}\n\
             Tags: {tags}\n\
             User note: {note}\n\
             Candidate concepts:\n{candidate_text}\n\n\
             Available themes:\n{themes_text}\n\n\
             Choose the best existing themes for this content."
        )
    }
}

async fn choose_relevant_themes(
    db: Arc<Database>,
    repo: &Repository,
    content: &CapturedContent,
) -> Result<Vec<String>, String> {
    let theme_pages = repo
        .get_wiki_pages_by_type("theme")
        .map_err(|e| format!("Failed to load themes: {}", e))?;
    if theme_pages.is_empty() {
        return Ok(Vec::new());
    }

    let candidates = repo
        .get_content_concept_candidates(&content.id)
        .map_err(|e| format!("Failed to load content concept candidates: {}", e))?;
    let locale = crate::locale::resolve_locale(&db);
    let system = build_theme_match_system_prompt(&locale);
    let user = build_theme_match_user_message(content, &theme_pages, &candidates, &locale);
    let raw = call_ai(db, &system, &user, 320).await?;
    let json = parse_ai_json(&raw)?;
    let valid_ids: HashSet<String> = theme_pages.iter().map(|page| page.id.clone()).collect();

    Ok(json
        .get("theme_ids")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .filter(|theme_id| valid_ids.contains(*theme_id))
        .take(3)
        .map(|theme_id| theme_id.to_string())
        .collect())
}

fn sync_theme_membership_edges(
    repo: &Repository,
    structured_page_ids: &[String],
    theme_ids: &[String],
) -> Result<(), String> {
    for page_id in structured_page_ids {
        let Some(page) = repo
            .get_wiki_page_by_id(page_id)
            .map_err(|e| format!("Failed to inspect structured page {}: {}", page_id, e))?
        else {
            continue;
        };
        if matches!(
            page.page_type.as_str(),
            "source" | "qa" | "theme" | "dashboard"
        ) {
            continue;
        }

        repo.delete_edges_for_page_with_relation(page_id, THEME_EDGE_RELATION)
            .map_err(|e| format!("Failed to clear theme edges for {}: {}", page.title, e))?;

        for theme_id in theme_ids {
            if theme_id == page_id {
                continue;
            }
            repo.save_wiki_edge(page_id, theme_id, THEME_EDGE_RELATION, 1.0)
                .map_err(|e| format!("Failed to save theme edge for {}: {}", page.title, e))?;
        }
    }

    Ok(())
}

async fn attach_relevant_themes(
    db: Arc<Database>,
    repo: &Repository,
    content: &CapturedContent,
    structured_page_ids: &[String],
) -> Result<Vec<String>, String> {
    if structured_page_ids.is_empty() {
        return Ok(Vec::new());
    }

    let theme_ids = choose_relevant_themes(db, repo, content).await?;
    sync_theme_membership_edges(repo, structured_page_ids, &theme_ids)?;
    Ok(theme_ids)
}

/// Call AI using the project's existing multi-provider infrastructure.
/// Reuses the same provider/model resolution as spawn_summary_task.
async fn call_ai(
    db: Arc<Database>,
    system_prompt: &str,
    user_message: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let repo = Repository::new(db.clone());

    let provider_str = repo
        .get_setting("ai_provider")
        .ok()
        .flatten()
        .unwrap_or_else(|| "anthropic".to_string());

    // Try OAuth paths first (is_deep=true: use strong models for wiki compilation & Q&A)
    if provider_str == "openai" {
        if let Some(result) = crate::ai::attention_analyzer::try_codex_call(
            db.clone(),
            system_prompt,
            user_message,
            0.3,
            true,
        )
        .await
        {
            return result;
        }
    }

    if provider_str == "google" {
        if let Some(result) = crate::ai::attention_analyzer::try_gemini_call(
            db.clone(),
            system_prompt,
            user_message,
            0.3,
            true,
        )
        .await
        {
            return result;
        }
    }

    // API key path
    let provider_key = format!("ai_api_key_{}", provider_str);
    let api_key = repo
        .get_setting(&provider_key)
        .ok()
        .flatten()
        .or_else(|| repo.get_setting("ai_api_key").ok().flatten())
        .unwrap_or_default();

    if api_key.is_empty() {
        return Err("AI API Key not configured".to_string());
    }

    let model = repo
        .get_setting("ai_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());

    let provider = crate::ai::attention_analyzer::AnalysisProvider::from_str(&provider_str);
    crate::ai::attention_analyzer::call_analysis_api(
        &provider,
        &api_key,
        &model,
        system_prompt,
        user_message,
        max_tokens,
    )
    .await
}

/// Parse JSON from AI response, stripping markdown code blocks if present.
fn parse_ai_json(raw: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw.trim();
    let cleaned = if trimmed.starts_with("```") {
        let without_prefix = if let Some(rest) = trimmed.strip_prefix("```json") {
            rest
        } else {
            &trimmed[3..]
        };
        without_prefix
            .strip_suffix("```")
            .unwrap_or(without_prefix)
            .trim()
    } else {
        trimmed
    };
    serde_json::from_str(cleaned).map_err(|e| {
        format!(
            "JSON parse failed: {} — raw: {}",
            e,
            &cleaned[..cleaned.len().min(200)]
        )
    })
}

/// Assess whether a content item has knowledge value.
/// Returns (should_compile, knowledge_score, reason).
pub async fn assess_content(
    db: Arc<Database>,
    content: &CapturedContent,
) -> Result<(bool, f64, String), String> {
    let locale = crate::locale::resolve_locale(&db);
    let system = wiki_prompts::assessment_system_prompt(&locale);
    let user = wiki_prompts::assessment_user_message(
        content.content_type.as_str(),
        content
            .clean_content
            .as_deref()
            .or(content.raw_text.as_deref())
            .unwrap_or(""),
        content.summary.as_deref().unwrap_or(""),
        content.user_note.as_deref().unwrap_or(""),
        content.source_url.as_deref().unwrap_or(""),
        &content.source_app,
        &locale,
    );

    let raw = call_ai(db, &system, &user, 256).await?;
    let json = parse_ai_json(&raw)?;

    let should = json
        .get("should_compile")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let score = json
        .get("knowledge_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let reason = json
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    Ok((should, score, reason))
}

/// Compile a content item into the wiki (two-stage process).
/// Returns the list of page IDs touched.
pub async fn compile_content(
    db: Arc<Database>,
    content: &CapturedContent,
) -> Result<Vec<String>, String> {
    let repo = Repository::new(db.clone());
    let current_hash = compute_content_hash_for_repo(&repo, content);
    let content_text = content
        .clean_content
        .as_deref()
        .or(content.raw_text.as_deref())
        .unwrap_or("");
    let content_summary = content.summary.as_deref().unwrap_or("");
    let content_tags = content.tags.as_deref().unwrap_or("");
    let user_note = content.user_note.as_deref().unwrap_or("");

    // --- Stage 1: Discovery ---
    let locale = crate::locale::resolve_locale(&db);
    let existing_pages = collect_relevant_page_summaries(
        &repo,
        content_text,
        content_summary,
        content_tags,
        user_note,
    )?;

    let discover_system = wiki_prompts::compile_discover_system_prompt(&locale);
    let discover_user = wiki_prompts::compile_discover_user_message(
        content_text,
        content_summary,
        content_tags,
        user_note,
        &existing_pages,
        &locale,
    );

    let discover_raw = call_ai(db.clone(), &discover_system, &discover_user, 1024).await?;
    let discover_json = parse_ai_json(&discover_raw)?;

    let creates = discover_json
        .get("creates")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let updates = discover_json
        .get("updates")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if creates.is_empty() && updates.is_empty() {
        log::info!(
            "Wiki compile: no pages to create or update for {}",
            content.id
        );
        return Ok(vec![]);
    }

    let mut touched_ids = Vec::new();
    let execute_system = wiki_prompts::compile_execute_system_prompt(&locale);
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // --- Stage 2: Execute creates ---
    for create_item in &creates {
        let title = create_item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let page_type = create_item
            .get("page_type")
            .and_then(|v| v.as_str())
            .unwrap_or("concept");

        let execute_user = wiki_prompts::compile_execute_create_message(
            content_text,
            content_summary,
            user_note,
            title,
            page_type,
            &locale,
        );

        let execute_raw = call_ai(db.clone(), &execute_system, &execute_user, 2048).await?;
        let page_json = parse_ai_json(&execute_raw)?;

        let page_id = uuid::Uuid::new_v4().to_string();
        let page_title = page_json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(title);
        let slug = slugify(page_title);
        let body = page_json
            .get("body_markdown")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let summary = page_json
            .get("summary")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let tags = page_json.get("tags").map(|v| v.to_string());
        let pt = page_json
            .get("page_type")
            .and_then(|v| v.as_str())
            .unwrap_or(page_type);

        // Ensure slug is unique
        let final_slug = if repo
            .get_wiki_page_by_slug(&slug)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            format!("{}-{}", slug, &page_id[..8])
        } else {
            slug
        };

        let page = WikiPage {
            id: page_id.clone(),
            title: page_title.to_string(),
            slug: final_slug,
            page_type: pt.to_string(),
            body_markdown: body.to_string(),
            summary,
            tags,
            status: "active".to_string(),
            confidence: 1.0,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_compiled_at: Some(now.clone()),
            source_message_id: None,
        };
        repo.save_wiki_page(&page)
            .map_err(|e| format!("Failed to save page: {}", e))?;
        repo.add_page_source(&page_id, &content.id, &current_hash)
            .map_err(|e| format!("Failed to save source relation: {}", e))?;

        // Note: we intentionally no longer process an `edges` field from the
        // AI response. Edges are computed deterministically from tags by
        // `link_pages_by_shared_tags` (TF-IDF cosine similarity). Keeping
        // the old AI-generated edges here meant two mechanisms wrote into
        // the same `relation = 'related'` slot with conflicting weights
        // (AI: fixed 1.0, TF-IDF: continuous 0.3-0.9), polluting the graph.

        touched_ids.push(page_id);
        log::info!("Wiki: created page \"{}\" ({})", page_title, pt);
    }

    // --- Stage 2: Execute updates ---
    for update_item in &updates {
        let page_id = update_item
            .get("page_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if page_id.is_empty() {
            continue;
        }

        let existing_page = match repo
            .get_wiki_page_by_id(page_id)
            .map_err(|e| e.to_string())?
        {
            Some(p) => p,
            None => {
                log::warn!(
                    "Wiki compile: page {} not found for update, skipping",
                    page_id
                );
                continue;
            }
        };

        // Get source stats for this page
        let sources = repo
            .get_sources_for_page(page_id)
            .map_err(|e| e.to_string())?;
        let active_count = sources
            .iter()
            .filter(|s| s.source_status == "active")
            .count();
        let stale_count = sources
            .iter()
            .filter(|s| s.source_status == "stale")
            .count();

        let execute_user = wiki_prompts::compile_execute_update_message(
            content_text,
            content_summary,
            user_note,
            &existing_page.body_markdown,
            &existing_page.title,
            active_count,
            stale_count,
            &locale,
        );

        let execute_raw = call_ai(db.clone(), &execute_system, &execute_user, 2048).await?;
        let page_json = parse_ai_json(&execute_raw)?;

        let body = page_json
            .get("body_markdown")
            .and_then(|v| v.as_str())
            .unwrap_or(&existing_page.body_markdown);
        let summary = page_json
            .get("summary")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing_page.summary.clone());
        let tags = page_json
            .get("tags")
            .map(|v| v.to_string())
            .or(existing_page.tags.clone());

        let updated_page = WikiPage {
            id: page_id.to_string(),
            title: existing_page.title.clone(),
            slug: existing_page.slug.clone(),
            page_type: existing_page.page_type.clone(),
            body_markdown: body.to_string(),
            summary,
            tags,
            status: "active".to_string(),
            confidence: existing_page.confidence,
            created_at: existing_page.created_at.clone(),
            updated_at: now.clone(),
            last_compiled_at: Some(now.clone()),
            source_message_id: existing_page.source_message_id.clone(),
        };
        repo.update_wiki_page(&updated_page)
            .map_err(|e| format!("Failed to update page: {}", e))?;
        repo.add_page_source(page_id, &content.id, &current_hash)
            .map_err(|e| format!("Failed to save source relation: {}", e))?;

        // Note: AI-generated edges are no longer processed here — see the
        // matching comment in the create branch above. Edges live entirely
        // in `link_pages_by_shared_tags`, which uses TF-IDF similarity.

        touched_ids.push(page_id.to_string());
        log::info!("Wiki: updated page \"{}\"", existing_page.title);
    }

    Ok(touched_ids)
}

/// Auto-compile: assess + compile if worthy. Updates hashes.
pub async fn auto_compile(db: Arc<Database>, content_id: &str) -> Result<(), String> {
    let repo = Repository::new(db.clone());
    let content = repo
        .get_content_by_id(content_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Content {} not found", content_id))?;

    let current_hash = compute_content_hash_for_repo(&repo, &content);

    // Check if already assessed at this version
    if content.wiki_assessed_hash.as_deref() == Some(&current_hash) {
        return Ok(());
    }

    // Acquire compile lock
    if !repo
        .acquire_compile_lock(content_id, &current_hash)
        .map_err(|e| e.to_string())?
    {
        log::info!("Wiki compile lock busy for {}, skipping", content_id);
        return Ok(());
    }

    // Assess
    let (should_compile, score, reason) = match assess_content(db.clone(), &content).await {
        Ok(result) => result,
        Err(e) => {
            let _ = repo.release_compile_lock(content_id, "error", None, None, Some(&e));
            return Err(e);
        }
    };

    log::info!(
        "Wiki assess {}: score={:.2}, should={}, reason={}",
        content_id,
        score,
        should_compile,
        reason
    );

    if !should_compile || score < 0.5 {
        // Not worth compiling — update assessed hash to avoid re-assessment
        let _ = repo.update_content_assessed_hash(content_id, &current_hash);
        let _ = repo.release_compile_lock(content_id, "skipped", None, None, None);
        return Ok(());
    }

    // Compile
    match compile_content(db.clone(), &content).await {
        Ok(mut touched_ids) => {
            let structured_page_ids = get_structured_page_ids_for_content(&repo, &content.id)?;
            let theme_page_ids =
                attach_relevant_themes(db.clone(), &repo, &content, &structured_page_ids)
                    .await
                    .unwrap_or_default();

            for theme_page_id in theme_page_ids {
                if !touched_ids.contains(&theme_page_id) {
                    touched_ids.push(theme_page_id);
                }
            }

            let touched_ids = if touched_ids.is_empty() {
                create_manual_source_page(&repo, &content, &current_hash)?
            } else {
                touched_ids
            };
            let _ = workspace::sync_wiki_pages_to_workspace_drafts(&repo, &touched_ids);
            let pages_json = serde_json::to_string(&touched_ids).unwrap_or_default();
            let _ = repo.update_content_compile_hash(content_id, &current_hash);
            let _ =
                repo.release_compile_lock(content_id, "completed", Some(&pages_json), None, None);
            log::info!(
                "Wiki compile done for {}: {} pages touched",
                content_id,
                touched_ids.len()
            );
            Ok(())
        }
        Err(e) => {
            // Don't update compile_hash on failure — will retry next time
            let _ = repo.update_content_assessed_hash(content_id, &current_hash);
            let _ = repo.release_compile_lock(content_id, "error", None, None, Some(&e));
            Err(e)
        }
    }
}

/// Manual compile: skip assessment, compile directly. Updates both hashes.
pub async fn manual_compile(db: Arc<Database>, content_id: &str) -> Result<Vec<String>, String> {
    let repo = Repository::new(db.clone());
    let mut content = repo
        .get_content_by_id(content_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Content {} not found", content_id))?;
    let is_imported_file = is_file_import_source(&content);

    if is_imported_file {
        content = ensure_content_metadata(db.clone(), &content).await?;
    }

    let current_hash = compute_content_hash_for_repo(&repo, &content);

    // Acquire compile lock
    let mut lock_acquired = repo
        .acquire_compile_lock(content_id, &current_hash)
        .map_err(|e| e.to_string())?;
    if !lock_acquired {
        let cleaned = repo
            .cleanup_stale_compile_lock_for_content(content_id, 120)
            .map_err(|e| e.to_string())?;
        if cleaned > 0 {
            lock_acquired = repo
                .acquire_compile_lock(content_id, &current_hash)
                .map_err(|e| e.to_string())?;
        }
    }
    if !lock_acquired {
        return Err("Compilation in progress, please try again later".to_string());
    }

    let source_page_ids = if is_imported_file {
        create_manual_source_page(&repo, &content, &current_hash)?
    } else {
        Vec::new()
    };

    if is_imported_file {
        let mut touched_ids = source_page_ids.clone();
        clear_existing_concept_sources_for_content(&repo, &content.id)?;
        let concept_page_ids = upsert_promoted_candidate_pages(&repo, &content)?;
        let structured_page_ids = get_structured_page_ids_for_content(&repo, &content.id)?;
        let theme_page_ids =
            attach_relevant_themes(db.clone(), &repo, &content, &structured_page_ids)
                .await
                .unwrap_or_default();
        let _ = sync_source_derivation_edges(&repo, &source_page_ids, &structured_page_ids);

        for page_id in concept_page_ids
            .into_iter()
            .chain(structured_page_ids.into_iter())
        {
            if !touched_ids.contains(&page_id) {
                touched_ids.push(page_id);
            }
        }
        for theme_page_id in theme_page_ids {
            if !touched_ids.contains(&theme_page_id) {
                touched_ids.push(theme_page_id);
            }
        }

        let _ = workspace::sync_wiki_pages_to_workspace_drafts(&repo, &touched_ids);

        let pages_json = serde_json::to_string(&touched_ids).unwrap_or_default();
        let _ = repo.update_content_compile_hash(content_id, &current_hash);
        let _ = repo.release_compile_lock(content_id, "completed", Some(&pages_json), None, None);
        return Ok(touched_ids);
    }

    match compile_content(db.clone(), &content).await {
        Ok(mut touched_ids) => {
            let structured_page_ids = get_structured_page_ids_for_content(&repo, &content.id)?;
            let theme_page_ids =
                attach_relevant_themes(db.clone(), &repo, &content, &structured_page_ids)
                    .await
                    .unwrap_or_default();
            if is_imported_file {
                let _ = sync_source_derivation_edges(&repo, &source_page_ids, &structured_page_ids);
            }

            for source_page_id in &source_page_ids {
                if !touched_ids.contains(source_page_id) {
                    touched_ids.push(source_page_id.clone());
                }
            }
            for structured_page_id in &structured_page_ids {
                if !touched_ids.contains(structured_page_id) {
                    touched_ids.push(structured_page_id.clone());
                }
            }
            for theme_page_id in &theme_page_ids {
                if !touched_ids.contains(theme_page_id) {
                    touched_ids.push(theme_page_id.clone());
                }
            }
            if touched_ids.is_empty() {
                touched_ids = create_manual_source_page(&repo, &content, &current_hash)?;
            }

            let _ = workspace::sync_wiki_pages_to_workspace_drafts(&repo, &touched_ids);
            let pages_json = serde_json::to_string(&touched_ids).unwrap_or_default();
            let _ = repo.update_content_compile_hash(content_id, &current_hash);
            let _ =
                repo.release_compile_lock(content_id, "completed", Some(&pages_json), None, None);
            Ok(touched_ids)
        }
        Err(e) => {
            let _ = repo.release_compile_lock(content_id, "error", None, None, Some(&e));
            Err(e)
        }
    }
}

/// Handle content deletion: update source status and page confidence.
pub fn on_content_deleted(db: Arc<Database>, content_id: &str) -> Result<(), String> {
    let repo = Repository::new(db);

    // Mark all sources from this content as deleted
    repo.update_source_status_by_content(content_id, "deleted")
        .map_err(|e| e.to_string())?;

    // Find all affected pages
    let affected = repo
        .get_pages_for_content(content_id)
        .map_err(|e| e.to_string())?;

    for source_record in &affected {
        let page_id = &source_record.page_id;

        // Recalculate confidence
        let confidence = repo
            .recalculate_page_confidence(page_id)
            .map_err(|e| e.to_string())?;

        let (active, _total) = repo
            .count_active_sources(page_id)
            .map_err(|e| e.to_string())?;

        if active > 0 {
            // Has remaining sources — mark for recompile.
            // Note: we intentionally do NOT generate a lint result here.
            // The user just deleted the content themselves, so pestering them
            // with a "source was deleted" notification on the Insight page is
            // noise, not signal. The page status change is enough — they can
            // still see affected pages in the knowledge base list if they care.
            let _ = repo.update_wiki_page_status(page_id, "needs_recompile", confidence);
        } else {
            // No active sources — tombstone. Same rationale: no lint generated.
            let _ = repo.update_wiki_page_status(page_id, "draft", 0.3);
        }
    }

    Ok(())
}

pub fn demote_low_support_concepts(db: Arc<Database>) -> Result<usize, String> {
    let repo = Repository::new(db);
    let concept_settings = load_concept_promotion_settings(&repo);
    let concept_pages = repo
        .get_wiki_pages_by_type("concept")
        .map_err(|e| e.to_string())?;
    let mut removed = 0usize;

    for page in concept_pages {
        if page
            .source_message_id
            .as_deref()
            .is_some_and(|source| source.starts_with(LOCAL_WIKI_SOURCE_PREFIX))
        {
            continue;
        }
        let (active_sources, _) = repo
            .count_active_sources(&page.id)
            .map_err(|e| e.to_string())?;
        let active_days = repo
            .count_active_source_days_for_page(&page.id)
            .map_err(|e| e.to_string())?;
        if active_sources >= concept_settings.min_sources
            && active_days >= concept_settings.min_days
        {
            continue;
        }

        let _ = workspace::remove_wiki_page_workspace_draft(&repo, &page);
        repo.delete_wiki_page_fully(&page.id)
            .map_err(|e| format!("Failed to demote low-support concept {}: {}", page.title, e))?;
        removed += 1;
    }

    if removed > 0 {
        log::info!(
            "Wiki concept cleanup: demoted {} low-support concept pages",
            removed
        );
    }

    Ok(removed)
}

/// Public wrapper for call_ai (used by wiki commands).
pub async fn call_ai_pub(
    db: Arc<Database>,
    system_prompt: &str,
    user_message: &str,
    max_tokens: u32,
) -> Result<String, String> {
    call_ai(db, system_prompt, user_message, max_tokens).await
}

/// Public wrapper for parse_ai_json (used by wiki commands).
pub fn parse_ai_json_pub(raw: &str) -> Result<serde_json::Value, String> {
    parse_ai_json(raw)
}

/// Link pages into a "related" graph using TF-IDF weighted cosine
/// similarity over their tags.
///
/// The old implementation connected any two pages that shared at least
/// one tag, which produced an exploding graph (988 pairs over 151 pages
/// in one real dataset) because common tags like "AI" or "agent" forced
/// every page touching those topics into a near-complete subgraph.
///
/// This version:
///
/// 1. Computes an IDF score for every tag — tags that appear on many
///    pages get a low weight automatically, so there's no manual
///    "stop word" list to maintain.
/// 2. Represents each page as a sparse TF-IDF vector over its tags.
/// 3. Scores every page pair by cosine similarity.
/// 4. Keeps only pairs with similarity >= SIM_THRESHOLD, and caps each
///    page at TOP_K neighbors (whichever side picks the edge first —
///    we dedupe via a canonical (min, max) ordering so each pair lands
///    as a single undirected edge).
///
/// The edge weight stored in the database is the cosine similarity
/// itself (between 0 and 1), which the frontend can use to modulate
/// stroke opacity or spring stiffness in the force-directed layout.
///
/// Complexity: O(n² · avg_tags_per_page). For n=150 this is a few
/// hundred thousand hash lookups — well under a second even in debug
/// builds.
pub fn link_pages_by_shared_tags(db: Arc<Database>) -> Result<usize, String> {
    use std::collections::HashMap;

    /// Maximum number of related pages to keep per node. Prevents
    /// any single page from becoming a super-node even if it's
    /// legitimately similar to many others.
    const TOP_K: usize = 8;

    /// Minimum cosine similarity required for two pages to be linked.
    /// 0.3 corresponds roughly to "meaningful topic overlap after
    /// down-weighting common tags".
    const SIM_THRESHOLD: f64 = 0.3;

    let repo = Repository::new(db);
    let pages = repo
        .get_all_wiki_pages(1000, 0)
        .map_err(|e| e.to_string())?;

    // Parse and normalize tags. A page with no usable tags is excluded
    // from the graph — there's nothing to compare it against.
    let page_tags: Vec<(String, Vec<String>)> = pages
        .iter()
        .filter_map(|p| {
            let tags_str = p.tags.as_deref()?;
            let tags: Vec<String> = serde_json::from_str(tags_str).unwrap_or_default();
            let mut normalized: Vec<String> = tags
                .iter()
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect();
            normalized.sort();
            normalized.dedup();
            if normalized.is_empty() {
                None
            } else {
                Some((p.id.clone(), normalized))
            }
        })
        .collect();

    let n = page_tags.len();
    if n < 2 {
        log::info!("Wiki tag-linking: fewer than 2 tagged pages, skipping");
        return Ok(0);
    }

    // --- Step 1: IDF for every tag -----------------------------------
    // IDF(t) = ln((N + 1) / (df(t) + 1)) — rare tags get a higher weight.
    // Adding 1 to numerator and denominator smooths the distribution
    // and guarantees a non-negative score even when df == N.
    let mut doc_freq: HashMap<&str, usize> = HashMap::new();
    for (_id, tags) in &page_tags {
        for t in tags {
            *doc_freq.entry(t.as_str()).or_insert(0) += 1;
        }
    }
    let total = n as f64;
    let idf: HashMap<&str, f64> = doc_freq
        .iter()
        .map(|(t, df)| {
            let score = ((total + 1.0) / (*df as f64 + 1.0)).ln();
            (*t, score)
        })
        .collect();

    // --- Step 2: TF-IDF vector per page ------------------------------
    // Tags are unique per page (we dedup'd above), so TF is always 1
    // and the vector value is just the IDF of the tag.
    let vectors: Vec<HashMap<&str, f64>> = page_tags
        .iter()
        .map(|(_id, tags)| {
            tags.iter()
                .filter_map(|t| idf.get(t.as_str()).map(|w| (t.as_str(), *w)))
                .collect()
        })
        .collect();

    // Precompute norms so cosine similarity is a single dot product.
    let norms: Vec<f64> = vectors
        .iter()
        .map(|v| v.values().map(|w| w * w).sum::<f64>().sqrt())
        .collect();

    // --- Step 3: pairwise cosine similarity, keep candidates above threshold ---
    // For each page i, collect its neighbors with sim >= SIM_THRESHOLD,
    // then keep the TOP_K most similar. The resulting edges from both
    // sides are merged through a canonical (min_idx, max_idx) tuple
    // so each unordered pair is inserted exactly once.
    let mut selected: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut edge_weight: HashMap<(usize, usize), f64> = HashMap::new();

    for i in 0..n {
        if norms[i] == 0.0 {
            continue;
        }
        let mut neighbors: Vec<(usize, f64)> = Vec::new();
        for j in 0..n {
            if i == j || norms[j] == 0.0 {
                continue;
            }
            // Iterate over the shorter of the two vectors for efficiency
            let (shorter, longer) = if vectors[i].len() <= vectors[j].len() {
                (&vectors[i], &vectors[j])
            } else {
                (&vectors[j], &vectors[i])
            };
            let dot: f64 = shorter
                .iter()
                .filter_map(|(t, w_a)| longer.get(t).map(|w_b| w_a * w_b))
                .sum();
            let sim = dot / (norms[i] * norms[j]);
            if sim >= SIM_THRESHOLD {
                neighbors.push((j, sim));
            }
        }
        // Keep top-K most similar
        neighbors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        neighbors.truncate(TOP_K);

        for (j, sim) in neighbors {
            let key = if i < j { (i, j) } else { (j, i) };
            selected.insert(key);
            // Store the max weight we've seen for this pair (symmetric
            // in theory, but shielded against numerical drift).
            let entry = edge_weight.entry(key).or_insert(0.0);
            if sim > *entry {
                *entry = sim;
            }
        }
    }

    // --- Step 4: write edges (one row per undirected pair) -----------
    let mut count = 0usize;
    for (i, j) in &selected {
        let (id_a, _) = &page_tags[*i];
        let (id_b, _) = &page_tags[*j];
        let w = edge_weight.get(&(*i, *j)).copied().unwrap_or(0.0);
        let _ = repo.save_wiki_edge(id_a, id_b, "related", w);
        count += 1;
    }

    log::info!(
        "Wiki tag-linking: {} undirected edges across {} tagged pages (top-K={}, threshold={})",
        count,
        n,
        TOP_K,
        SIM_THRESHOLD
    );
    Ok(count)
}

/// Handle content update: mark sources as stale if hash changed.
pub fn on_content_updated(
    db: Arc<Database>,
    content_id: &str,
    new_hash: &str,
) -> Result<(), String> {
    let repo = Repository::new(db);

    let sources = repo
        .get_pages_for_content(content_id)
        .map_err(|e| e.to_string())?;

    for source_record in &sources {
        if source_record.compile_hash != new_hash && source_record.source_status == "active" {
            let _ = repo.update_source_status(&source_record.page_id, content_id, "stale");
            let _ = repo.update_wiki_page_status(
                &source_record.page_id,
                "needs_recompile",
                // Keep existing confidence for now
                1.0, // Will be recalculated on recompile
            );
        }
    }

    Ok(())
}
