/// Wiki knowledge base compilation engine.
///
/// Core operations:
/// - assess: evaluate if content has knowledge value
/// - compile: incrementally build wiki pages from content
/// - query: answer questions based on compiled wiki
/// - lint: health-check the wiki

use crate::storage::database::Database;
use crate::storage::models::{CapturedContent, WikiPage};
use crate::storage::repository::Repository;
use std::sync::Arc;

use super::wiki_prompts;

/// Compute a hash of the content's current state for change detection.
pub fn compute_content_hash(content: &CapturedContent) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Prefer clean_content for hash computation — ensures re-compilation when cleaned
    let text = content.clean_content.as_deref()
        .or(content.raw_text.as_deref())
        .unwrap_or("");
    text.hash(&mut hasher);
    content.summary.as_deref().unwrap_or("").hash(&mut hasher);
    content.tags.as_deref().unwrap_or("").hash(&mut hasher);
    content.digest.as_deref().unwrap_or("").hash(&mut hasher);
    content.user_note.as_deref().unwrap_or("").hash(&mut hasher);
    content.source_url.as_deref().unwrap_or("").hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
    let is_local_or_custom = provider_str == "custom" || provider_str == "ollama" || provider_str == "lmstudio";
    let provider_key = format!("ai_api_key_{}", provider_str);
    let api_key = repo
        .get_setting(&provider_key)
        .ok()
        .flatten()
        .or_else(|| repo.get_setting("ai_api_key").ok().flatten())
        .unwrap_or_default();

    if api_key.is_empty() && !is_local_or_custom {
        return Err("AI API Key not configured".to_string());
    }

    let model = repo
        .get_setting("ai_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

    let base_url = repo
        .get_setting("ai_custom_base_url")
        .ok()
        .flatten()
        .unwrap_or_default();

    let provider = crate::ai::attention_analyzer::AnalysisProvider::from_str_with_base(
        &provider_str,
        &base_url,
    );
    crate::ai::attention_analyzer::call_analysis_api(
        &provider,
        &api_key,
        &model,
        system_prompt,
        user_message,
        max_tokens,
        true,
    )
    .await
}

/// Parse JSON from an AI response. Robust to three failure modes
/// commonly seen with relay-served models:
///
/// 1. ```json ...``` markdown wrappers — strip them.
/// 2. Conversational prose followed by JSON ("Sure, here you go: {...}")
///    — find the first balanced `{...}` and parse just that.
/// 3. Garbage on both sides — same as (2), look for the JSON island.
///
/// If none of the above yields valid JSON, return the parse error.
fn parse_ai_json(raw: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw.trim();

    // Strip markdown code fences if present.
    let stripped = if trimmed.starts_with("```") {
        let without_prefix = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```JSON"))
            .unwrap_or(&trimmed[3..]);
        without_prefix
            .strip_suffix("```")
            .unwrap_or(without_prefix)
            .trim()
    } else {
        trimmed
    };

    // First attempt: parse the whole string as-is.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stripped) {
        return Ok(v);
    }

    // Fallback: scan for the first balanced JSON object. Naive bracket
    // counting is fine here because we don't try to parse strings —
    // serde_json does that on the candidate slice. We just need to
    // find a candidate that ends at the right `}`.
    if let Some(extracted) = extract_balanced_json(stripped) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&extracted) {
            return Ok(v);
        }
    }

    // Give up — surface the original parse error with a char-safe preview.
    let err = serde_json::from_str::<serde_json::Value>(stripped).unwrap_err();
    let preview: String = stripped.chars().take(200).collect();
    Err(format!("JSON parse failed: {} — raw: {}", err, preview))
}

/// Extract the first balanced `{...}` substring, respecting string
/// literals and escapes so braces inside JSON strings don't throw off
/// the counter.
fn extract_balanced_json(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut i = start;

    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            escape = false;
        } else if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        // Slice on a UTF-8 char boundary by going through &str
                        return Some(s[start..=i].to_string());
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
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
        content.clean_content.as_deref().or(content.raw_text.as_deref()).unwrap_or(""),
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
    let current_hash = compute_content_hash(content);
    let content_text = content.clean_content.as_deref().or(content.raw_text.as_deref()).unwrap_or("");
    let content_summary = content.summary.as_deref().unwrap_or("");
    let content_tags = content.tags.as_deref().unwrap_or("");
    let user_note = content.user_note.as_deref().unwrap_or("");

    // --- Stage 1: Discovery ---
    let locale = crate::locale::resolve_locale(&db);
    // Pre-filter the page index via FTS using the new content's own
    // summary + tags + opening text as the search signal. This replaces
    // dumping the entire wiki to the LLM on every capture.
    //
    // Fallback chain:
    //   1. FTS-pre-filtered candidates (top ~30)
    //   2. If FTS returns nothing (new topic or FTS unavailable), use the
    //      full summary list as before — keeps the legacy guarantee that
    //      Discovery can always see existing pages
    let mut fts_signal = String::new();
    fts_signal.push_str(content_summary);
    fts_signal.push(' ');
    fts_signal.push_str(content_tags);
    fts_signal.push(' ');
    fts_signal.extend(content_text.chars().take(200));

    let mut existing_pages: Vec<(String, String, String)> = repo
        .get_wiki_page_candidates(Some(&fts_signal), None, None, false, 30)
        .map_err(|e| format!("Failed to get page candidates: {}", e))?
        .into_iter()
        .map(|(id, title, summary, _created_at, _url)| (id, title, summary))
        .collect();

    if existing_pages.is_empty() {
        existing_pages = repo
            .get_wiki_page_summaries()
            .map_err(|e| format!("Failed to get page index: {}", e))?;
    }

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
        log::info!("Wiki compile: no pages to create or update for {}", content.id);
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
        let final_slug = if repo.get_wiki_page_by_slug(&slug).map_err(|e| e.to_string())?.is_some() {
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
        repo.save_wiki_page(&page).map_err(|e| format!("Failed to save page: {}", e))?;
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

        let existing_page = match repo.get_wiki_page_by_id(page_id).map_err(|e| e.to_string())? {
            Some(p) => p,
            None => {
                log::warn!("Wiki compile: page {} not found for update, skipping", page_id);
                continue;
            }
        };

        // Get source stats for this page
        let sources = repo.get_sources_for_page(page_id).map_err(|e| e.to_string())?;
        let active_count = sources.iter().filter(|s| s.source_status == "active").count();
        let stale_count = sources.iter().filter(|s| s.source_status == "stale").count();

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
            source_message_id: None,
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

    let current_hash = compute_content_hash(&content);

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
        content_id, score, should_compile, reason
    );

    if !should_compile || score < 0.5 {
        // Not worth compiling — update assessed hash to avoid re-assessment
        let _ = repo.update_content_assessed_hash(content_id, &current_hash);
        let _ = repo.release_compile_lock(content_id, "skipped", None, None, None);
        return Ok(());
    }

    // Compile
    match compile_content(db.clone(), &content).await {
        Ok(touched_ids) => {
            let pages_json = serde_json::to_string(&touched_ids).unwrap_or_default();
            let _ = repo.update_content_compile_hash(content_id, &current_hash);
            let _ = repo.release_compile_lock(
                content_id,
                "completed",
                Some(&pages_json),
                None,
                None,
            );
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
    let content = repo
        .get_content_by_id(content_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Content {} not found", content_id))?;

    let current_hash = compute_content_hash(&content);

    // Acquire compile lock
    if !repo
        .acquire_compile_lock(content_id, &current_hash)
        .map_err(|e| e.to_string())?
    {
        return Err("Compilation in progress, please try again later".to_string());
    }

    match compile_content(db.clone(), &content).await {
        Ok(touched_ids) => {
            let pages_json = serde_json::to_string(&touched_ids).unwrap_or_default();
            let _ = repo.update_content_compile_hash(content_id, &current_hash);
            let _ = repo.release_compile_lock(
                content_id,
                "completed",
                Some(&pages_json),
                None,
                None,
            );
            Ok(touched_ids)
        }
        Err(e) => {
            let _ = repo.release_compile_lock(content_id, "error", None, None, Some(&e));
            Err(e)
        }
    }
}

/// Handle content deletion: update source status and page confidence.
pub fn on_content_deleted(
    db: Arc<Database>,
    content_id: &str,
) -> Result<(), String> {
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
    let mut selected: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();
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
        neighbors.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_json() {
        let v = parse_ai_json(r#"{"page_ids": ["a", "b"]}"#).unwrap();
        assert_eq!(v["page_ids"][0], "a");
    }

    #[test]
    fn parses_json_in_markdown_fence() {
        let v = parse_ai_json("```json\n{\"page_ids\": [\"x\"]}\n```").unwrap();
        assert_eq!(v["page_ids"][0], "x");
    }

    #[test]
    fn parses_json_after_chinese_preamble() {
        // The exact failure mode we saw with the user's relay
        let raw = "根据你的反馈，那个不是你要找的。让我返回 JSON：\n{\"page_ids\": [\"abc\", \"def\"]}";
        let v = parse_ai_json(raw).unwrap();
        assert_eq!(v["page_ids"][0], "abc");
        assert_eq!(v["page_ids"][1], "def");
    }

    #[test]
    fn parses_json_with_braces_in_string_values() {
        // Bracket counting must not be fooled by braces inside strings
        let raw = r#"prelude {"answer": "this { is } tricky", "ok": true} trailing"#;
        let v = parse_ai_json(raw).unwrap();
        assert_eq!(v["answer"], "this { is } tricky");
    }

    #[test]
    fn parses_json_with_escaped_quotes() {
        let raw = r#"sure: {"answer": "she said \"hi\""}"#;
        let v = parse_ai_json(raw).unwrap();
        assert_eq!(v["answer"], "she said \"hi\"");
    }

    #[test]
    fn returns_error_for_pure_prose() {
        let result = parse_ai_json("你好！👋 有什么我可以帮你的吗？");
        assert!(result.is_err());
    }

    #[test]
    fn does_not_panic_on_cjk_in_error_preview() {
        // Regression for the byte-boundary panic
        let raw: String = "你好！".repeat(100);
        let _ = parse_ai_json(&raw);
    }
}
