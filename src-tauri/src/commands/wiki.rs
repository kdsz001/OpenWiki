use crate::ai::wiki_engine;
use crate::commands::capture::AppState;
use crate::storage::models::{WikiConversation, WikiLintResult, WikiPage};
use crate::storage::repository::Repository;
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, State};

// ===== Browse =====

#[tauri::command]
pub fn get_wiki_pages(
    state: State<'_, AppState>,
    page_type: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    let lim = limit.unwrap_or(100);
    let off = offset.unwrap_or(0);
    if let Some(pt) = page_type {
        repo.get_wiki_pages_by_type(&pt).map_err(|e| e.to_string())
    } else {
        repo.get_all_wiki_pages(lim, off).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_wiki_page(state: State<'_, AppState>, id: String) -> Result<Option<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_wiki_page_by_id(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_wiki(state: State<'_, AppState>, query: String) -> Result<Vec<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    repo.search_wiki_pages(&query, 20)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_wiki_stats(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_wiki_stats().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_wiki_page(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    repo.delete_edges_for_page(&id).map_err(|e| e.to_string())?;
    repo.delete_sources_for_page(&id)
        .map_err(|e| e.to_string())?;
    repo.delete_wiki_page(&id).map_err(|e| e.to_string())
}

// ===== Graph =====

#[tauri::command]
pub fn get_wiki_graph(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let repo = Repository::new(state.db.clone());
    let pages = repo.get_all_wiki_pages(500, 0).map_err(|e| e.to_string())?;
    let edges = repo.get_all_wiki_edges().map_err(|e| e.to_string())?;

    let nodes: Vec<serde_json::Value> = pages
        .iter()
        .map(|p| {
            let edge_count = edges
                .iter()
                .filter(|e| e.source_page_id == p.id || e.target_page_id == p.id)
                .count();
            serde_json::json!({
                "id": p.id,
                "title": p.title,
                "page_type": p.page_type,
                "status": p.status,
                "confidence": p.confidence,
                "edge_count": edge_count,
            })
        })
        .collect();

    let edge_data: Vec<serde_json::Value> = edges
        .iter()
        .map(|e| {
            serde_json::json!({
                "source": e.source_page_id,
                "target": e.target_page_id,
                "relation": e.relation,
                "weight": e.weight,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "nodes": nodes,
        "edges": edge_data,
    }))
}

// ===== Compile =====

#[tauri::command]
pub async fn compile_content_to_wiki(
    app: AppHandle,
    state: State<'_, AppState>,
    content_id: String,
) -> Result<Vec<String>, String> {
    let db = state.db.clone();
    let _ = app.emit("wiki-compile-progress", "compiling");

    match wiki_engine::manual_compile(db.clone(), &content_id).await {
        Ok(touched_ids) => {
            // Auto-link pages by shared tags after compilation
            let _ = wiki_engine::link_pages_by_shared_tags(db);
            let _ = app.emit("wiki-compile-complete", &touched_ids);
            Ok(touched_ids)
        }
        Err(e) => {
            let _ = app.emit("wiki-compile-error", &e);
            Err(e)
        }
    }
}

// ===== Q&A (3-stage: rewrite → retrieve → answer) =====

use crate::storage::models::{WikiChatMessage, WikiChatSession};

#[tauri::command]
pub async fn wiki_ask(
    state: State<'_, AppState>,
    session_id: String,
    question: String,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let repo = Repository::new(db.clone());
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Ensure session exists
    let sessions = repo.get_chat_sessions(100).map_err(|e| e.to_string())?;
    if !sessions.iter().any(|s| s.id == session_id) {
        let title: String = question.chars().take(30).collect();
        repo.create_chat_session(&session_id, Some(&title))
            .map_err(|e| e.to_string())?;
    }

    // Save user message
    let user_turn = repo
        .get_next_turn_index(&session_id)
        .map_err(|e| e.to_string())?;
    let user_msg = WikiChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.clone(),
        role: "user".to_string(),
        content: question.clone(),
        pages_used: None,
        source_mode: None,
        turn_index: user_turn,
        created_at: now.clone(),
    };
    repo.add_chat_message(&user_msg)
        .map_err(|e| e.to_string())?;

    // Build conversation context from recent turns. The real cap is the
    // character budget inside `build_conversation_context` — the turn
    // count is just a safety net so we don't walk a 5000-row table for
    // a chat that's been alive for months. Modern LLMs handle 200k+
    // context, so being generous here costs essentially nothing.
    let messages = repo
        .get_chat_messages(&session_id)
        .map_err(|e| e.to_string())?;
    let recent_context = build_conversation_context(&messages, 50);
    let follow_up_page_ids = resolve_follow_up_page_ids(&question, &messages);

    // Stage 0: Extract search keywords. We run this ALWAYS, not just on
    // multi-turn — even a first-turn question like "我之前保存了一个设计
    // 相关的 skill 是什么来着" carries 80%+ filler words that pollute the
    // FTS query. The LLM strips them down to "设计 skill", which the FTS
    // tokenizer can actually match against the index. On failure we fall
    // back to the raw question (no worse than the old behavior).
    let search_query = match rewrite_query(db.clone(), &question, &recent_context).await {
        Ok(q) => sanitize_keyword_output(&q).unwrap_or_else(|| question.clone()),
        Err(_) => question.clone(),
    };
    log::info!("Q&A search keywords: {}", search_query);

    // Pre-filter: detect a time window in the user's question, and use
    // the rewritten query as the FTS expression. This collapses what
    // used to be a "send the entire page index to the LLM every turn"
    // into "send ~30 SQL-pre-filtered candidates". See migration 014
    // for the FTS5 index.
    let now_utc = chrono::Utc::now();
    let today_iso = now_utc.format("%Y-%m-%d").to_string();
    let time_range = crate::ai::time_filter::detect_time_range(&question, now_utc);
    if let Some(ref tr) = time_range {
        log::info!("Q&A detected time range: {}", tr.label);
    }
    let date_start = time_range.as_ref().map(|t| t.iso_start());
    let date_end = time_range.as_ref().map(|t| t.iso_end());

    // Try FTS-pre-filtered candidates first. Limit 100 (not 50): FTS5
    // BM25 underweights pages where the user's query matches via common
    // CJK characters compared to pages with rare English tokens, so the
    // truly relevant CJK match can sit at rank 25+. A wider pool lets
    // Stage 1's semantic re-ranking surface those matches.
    let mut page_index: Vec<(String, String, String, String, Option<String>)> = repo
        .get_wiki_page_candidates(
            Some(&search_query),
            date_start.as_deref(),
            date_end.as_deref(),
            true, // exclude qa pages from Q&A retrieval
            100,
        )
        .map_err(|e| e.to_string())?;

    // Fallback: when FTS returned nothing AND no time filter is set,
    // the question is probably broad ("what do I care about"). Recent
    // pages are a sane default — recency is a usable proxy for "what
    // the user has been thinking about lately".
    if page_index.is_empty() && time_range.is_none() {
        page_index = repo
            .get_wiki_page_candidates(None, None, None, true, 30)
            .map_err(|e| e.to_string())?;
    }
    // Fallback for time-bound questions where FTS missed: drop the FTS
    // term but keep the date window. The LLM can still answer "what
    // was saved last week" from titles + dates alone.
    if page_index.is_empty() && time_range.is_some() {
        page_index = repo
            .get_wiki_page_candidates(None, date_start.as_deref(), date_end.as_deref(), true, 50)
            .map_err(|e| e.to_string())?;
    }

    // Stage 1: Retrieve relevant page IDs via AI (now from the small candidate set)
    let relevant_ids = if page_index.is_empty() {
        vec![]
    } else {
        match retrieve_relevant_pages(
            db.clone(),
            &search_query,
            &recent_context,
            &today_iso,
            &page_index,
        )
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                log::warn!("Q&A stage 1 (retrieve) failed: {}", e);
                vec![] // fall back to ai_only
            }
        }
    };
    let relevant_ids = merge_page_ids(follow_up_page_ids, relevant_ids);

    // Stage 2: Load full pages and answer
    let relevant_pages: Vec<(String, String, String)> = relevant_ids
        .iter()
        .filter_map(|id| {
            repo.get_wiki_page_by_id(id)
                .ok()
                .flatten()
                .filter(|p| p.status == "active" && p.confidence >= 0.5)
                .map(|p| (p.id, p.title, p.body_markdown))
        })
        .collect();

    let locale = crate::locale::resolve_locale(&db);
    let answer_system = crate::ai::wiki_prompts::query_answer_system_prompt(&locale);
    let answer_user = crate::ai::wiki_prompts::query_answer_user_message(
        &question,
        &recent_context,
        &today_iso,
        &relevant_pages,
        &page_index,
        &locale,
    );

    // 8192 tokens ≈ 6000 Chinese chars — generous ceiling that lets
    // the model fully unfold deep multi-section answers when warranted.
    // It's a cap, not a target: simple questions still get short
    // answers (the prompt enforces "length serves depth").
    let raw = wiki_engine::call_ai_pub(db.clone(), &answer_system, &answer_user, 8192).await?;

    // Parse response — graceful fallback
    let (answer, page_ids_used, source_mode, confidence) =
        match wiki_engine::parse_ai_json_pub(&raw) {
            Ok(json) => {
                // `reasoning` is the model's private scratchpad — it
                // forces step-by-step thinking before the visible answer.
                // We log it for debugging but never surface it to the UI.
                if let Some(r) = json.get("reasoning").and_then(|v| v.as_str()) {
                    let preview: String = r.chars().take(200).collect();
                    log::debug!("Q&A reasoning: {}", preview);
                }
                let a = strip_inline_page_ids(
                    json.get("answer").and_then(|v| v.as_str()).unwrap_or(&raw),
                );
                let pids: Vec<String> = json
                    .get("page_ids_used")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let sm = json
                    .get("source_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or(if pids.is_empty() {
                        "ai_only"
                    } else {
                        "knowledge_base"
                    })
                    .to_string();
                let c = json
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5);
                let inferred = infer_cited_page_ids(&a, &relevant_pages, &page_index);
                let merged_ids = merge_page_ids(pids, inferred);
                let normalized_mode = normalize_source_mode(&a, &sm, &merged_ids);
                (a, merged_ids, normalized_mode, c)
            }
            Err(_) => {
                // Malformed JSON — try to extract "answer" field via regex
                let extracted = strip_inline_page_ids(&extract_answer_from_malformed_json(&raw));
                let inferred = infer_cited_page_ids(&extracted, &relevant_pages, &page_index);
                let normalized_mode = normalize_source_mode(&extracted, "ai_only", &inferred);
                (extracted, inferred, normalized_mode, 0.3)
            }
        };

    // Save assistant message
    let asst_turn = repo
        .get_next_turn_index(&session_id)
        .map_err(|e| e.to_string())?;
    let pages_json = serde_json::to_string(&page_ids_used).unwrap_or_else(|_| "[]".to_string());
    let asst_msg = WikiChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.clone(),
        role: "assistant".to_string(),
        content: answer.clone(),
        pages_used: Some(pages_json.clone()),
        source_mode: Some(source_mode.clone()),
        turn_index: asst_turn,
        created_at: now.clone(),
    };
    repo.add_chat_message(&asst_msg)
        .map_err(|e| e.to_string())?;
    let _ = repo.touch_chat_session(&session_id);

    // Resolve page titles for frontend display
    let page_titles: Vec<serde_json::Value> = page_ids_used
        .iter()
        .filter_map(|id| {
            repo.get_wiki_page_by_id(id)
                .ok()
                .flatten()
                .map(|p| serde_json::json!({"id": p.id, "title": p.title}))
        })
        .collect();

    Ok(serde_json::json!({
        "message_id": asst_msg.id,
        "answer": answer,
        "pages_used": page_titles,
        "source_mode": source_mode,
        "confidence": confidence,
    }))
}

/// Sanitize the keyword output from Stage 0. Some models — especially
/// when served via OpenAI-compatible relays that auto-inject tool-use
/// formatting — return a JSON tool-call shape like
/// `[{"name":"WebSearch","parameters":{"query":"设计 skill"}}]`
/// instead of plain keywords. We don't want that as our FTS query.
///
/// Strategy:
/// 1. If the output starts with `{` or `[`, try to parse as JSON and
///    pull a "query"/"q"/"input" field out.
/// 2. If parsing fails or no query field, fall through to None and let
///    the caller use the raw user question.
/// 3. Strip surrounding whitespace and any obvious code fences.
///
/// Returns None when the output is unusable (caller decides fallback).
fn sanitize_keyword_output(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strip markdown fences if present
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(trimmed);

    // Plain text path — looks like keywords, return as-is
    if !stripped.starts_with('{') && !stripped.starts_with('[') {
        return Some(stripped.to_string());
    }

    // JSON path — try to find an embedded "query"/"q"/"input" string
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stripped) {
        if let Some(extracted) = find_query_field(&value) {
            return Some(extracted);
        }
    }
    // JSON-shaped but unparseable / no usable field — give up, caller falls back
    None
}

fn find_query_field(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            for key in &["query", "q", "input", "keywords", "search"] {
                if let Some(v) = map.get(*key) {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
            // Recurse into "parameters", "arguments", "args"
            for key in &["parameters", "arguments", "args"] {
                if let Some(v) = map.get(*key) {
                    if let Some(s) = find_query_field(v) {
                        return Some(s);
                    }
                }
            }
            None
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(find_query_field),
        _ => None,
    }
}

/// Belt-and-suspenders: even with a prompt that explicitly forbids them,
/// the model occasionally writes raw page UUIDs in the answer body. They
/// are pure noise to the user (the UI shows nice citation pills derived
/// from `page_ids_used`) and the long unbreakable strings overflow the
/// narrow chat sidebar — visually overlapping adjacent lines.
///
/// We strip any standalone `[uuid]` reference from the answer text. The
/// UUID itself is preserved in `page_ids_used` for the pill rendering.
fn strip_inline_page_ids(text: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"\s*\[[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\]",
        )
        .expect("regex compiles")
    });
    re.replace_all(text, "").into_owned()
}

/// Try to extract the "answer" field from malformed JSON.
/// Handles cases like: {"answer": "内容...", "page_ids_used": ...}
/// where the overall JSON is broken but the answer value is recoverable.
fn extract_answer_from_malformed_json(raw: &str) -> String {
    // Strategy 1: find "answer" key and extract its string value
    if let Some(start) = raw.find("\"answer\"") {
        let after_key = &raw[start + 8..]; // skip "answer"
                                           // Skip whitespace and colon
        let after_colon = after_key.trim_start();
        if let Some(rest) = after_colon.strip_prefix(':') {
            let rest = rest.trim_start();
            if rest.starts_with('"') {
                // Walk the string, handling escaped quotes
                let chars: Vec<char> = rest.chars().collect();
                let mut i = 1; // skip opening quote
                let mut result = String::new();
                while i < chars.len() {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        match chars[i + 1] {
                            'n' => result.push('\n'),
                            't' => result.push('\t'),
                            '"' => result.push('"'),
                            '\\' => result.push('\\'),
                            other => {
                                result.push('\\');
                                result.push(other);
                            }
                        }
                        i += 2;
                    } else if chars[i] == '"' {
                        break; // closing quote
                    } else {
                        result.push(chars[i]);
                        i += 1;
                    }
                }
                if !result.is_empty() {
                    return result;
                }
            }
        }
    }
    // Strategy 2: if nothing worked, strip obvious JSON wrapper
    raw.to_string()
}

/// Build conversation context string from recent messages.
///
/// Budget = 16000 chars (~12k tokens). At ~1500 chars per turn pair
/// that fits roughly 10+ real exchanges — enough for a substantive
/// thread, far short of any model's context window. The turn count is
/// just a safety net to avoid scanning huge histories; the budget is
/// the real cap.
fn build_conversation_context(messages: &[WikiChatMessage], max_turns: usize) -> String {
    let recent: Vec<&WikiChatMessage> = messages.iter().rev().take(max_turns * 2).collect();
    let mut parts = Vec::new();
    let mut budget = 16000i64;
    for msg in recent.iter().rev() {
        let role_label = if msg.role == "user" {
            "User"
        } else {
            "Assistant"
        };
        let content: String = msg.content.chars().take(budget.max(0) as usize).collect();
        budget -= content.len() as i64;
        parts.push(format!("{}: {}", role_label, content));
        if budget <= 0 {
            break;
        }
    }
    parts.join("\n")
}

fn resolve_follow_up_page_ids(question: &str, messages: &[WikiChatMessage]) -> Vec<String> {
    let compact: String = question.chars().filter(|c| !c.is_whitespace()).collect();
    let target_index = if compact.contains("第一个") || compact.contains("第1个") {
        Some(0usize)
    } else if compact.contains("第二个") || compact.contains("第2个") {
        Some(1usize)
    } else if compact.contains("第三个") || compact.contains("第3个") {
        Some(2usize)
    } else if compact.contains("最后一个") || compact.contains("最后那个") {
        Some(usize::MAX)
    } else {
        None
    };

    let Some(target_index) = target_index else {
        return Vec::new();
    };

    let page_ids = messages
        .iter()
        .rev()
        .find(|msg| msg.role == "assistant")
        .and_then(|msg| msg.pages_used.as_deref())
        .map(parse_page_ids)
        .unwrap_or_default();

    if page_ids.is_empty() {
        return Vec::new();
    }

    if target_index == usize::MAX {
        return page_ids.last().cloned().into_iter().collect();
    }

    page_ids.get(target_index).cloned().into_iter().collect()
}

fn parse_page_ids(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(id) if !id.is_empty() => Some(id.clone()),
            serde_json::Value::Object(obj) => {
                obj.get("id").and_then(|v| v.as_str()).map(str::to_string)
            }
            _ => None,
        })
        .collect()
}

fn merge_page_ids(primary: Vec<String>, secondary: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    primary
        .into_iter()
        .chain(secondary)
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect()
}

fn infer_cited_page_ids(
    answer: &str,
    relevant_pages: &[(String, String, String)],
    page_overview: &[(String, String, String, String, Option<String>)],
) -> Vec<String> {
    let titles: Vec<(String, String)> = relevant_pages
        .iter()
        .map(|(id, title, _)| (id.clone(), title.clone()))
        .chain(
            page_overview
                .iter()
                .map(|(id, title, _, _, _)| (id.clone(), title.clone())),
        )
        .collect();
    infer_cited_page_ids_from_titles(answer, &titles)
}

fn infer_cited_page_ids_from_titles(answer: &str, title_index: &[(String, String)]) -> Vec<String> {
    let mut titles = title_index.to_vec();
    titles.sort_by(|a, b| b.1.chars().count().cmp(&a.1.chars().count()));

    let mut strong = Vec::new();
    let mut weak = Vec::new();
    let mut seen = HashSet::new();

    for (id, title) in titles {
        if !seen.insert(id.clone()) {
            continue;
        }
        match title_mention_strength(answer, &title) {
            MentionStrength::Strong => strong.push(id),
            MentionStrength::Weak => weak.push(id),
            MentionStrength::None => {}
        }
    }

    if !strong.is_empty() {
        strong
    } else {
        weak
    }
}

enum MentionStrength {
    None,
    Weak,
    Strong,
}

fn title_mention_strength(answer: &str, title: &str) -> MentionStrength {
    let title = title.trim();
    if title.is_empty() {
        return MentionStrength::None;
    }

    let strong_patterns = [
        format!("**{}**", title),
        format!("`{}`", title),
        format!("「{}」", title),
        format!("【{}】", title),
        format!("[{}]", title),
    ];
    if strong_patterns
        .iter()
        .any(|pattern| answer.contains(pattern))
    {
        return if title_is_strong_matchable(title) {
            MentionStrength::Strong
        } else {
            MentionStrength::None
        };
    }

    if title.chars().all(|c| c.is_ascii()) {
        let title_lower = title.to_lowercase();
        if answer.to_lowercase().contains(&title_lower) && ascii_title_is_matchable(title) {
            MentionStrength::Weak
        } else {
            MentionStrength::None
        }
    } else if answer.contains(title) && non_ascii_title_is_matchable(title) {
        MentionStrength::Weak
    } else {
        MentionStrength::None
    }
}

fn title_is_strong_matchable(title: &str) -> bool {
    if title.chars().all(|c| c.is_ascii()) {
        return ascii_title_is_matchable(title);
    }
    non_ascii_meaningful_char_count(title) >= 3 || title_has_distinctive_marker(title)
}

fn ascii_title_is_matchable(title: &str) -> bool {
    title.len() >= 8
        || title.contains('-')
        || title.contains('_')
        || title.contains(' ')
        || title.chars().any(|c| c.is_ascii_digit())
}

fn non_ascii_title_is_matchable(title: &str) -> bool {
    non_ascii_meaningful_char_count(title) >= 4 || title_has_distinctive_marker(title)
}

fn non_ascii_meaningful_char_count(title: &str) -> usize {
    title.chars().filter(|c| !c.is_whitespace()).count()
}

fn title_has_distinctive_marker(title: &str) -> bool {
    title.contains('-')
        || title.contains('_')
        || title.contains(' ')
        || title.chars().any(|c| c.is_ascii_digit())
}

fn normalize_source_mode(answer: &str, source_mode: &str, page_ids_used: &[String]) -> String {
    if page_ids_used.is_empty() {
        return source_mode.to_string();
    }

    if answer.contains("[AI supplement]") || answer.contains("[AI 补充]") {
        return "mixed".to_string();
    }

    if source_mode == "mixed" {
        "mixed".to_string()
    } else {
        "knowledge_base".to_string()
    }
}

fn repair_chat_message_metadata(
    repo: &Repository,
    messages: &mut [WikiChatMessage],
) -> Result<(), Box<dyn std::error::Error>> {
    let title_index = repo.get_active_wiki_page_titles()?;

    for msg in messages.iter_mut() {
        if msg.role != "assistant" {
            continue;
        }

        let existing_ids = msg
            .pages_used
            .as_deref()
            .map(parse_page_ids)
            .unwrap_or_default();
        let needs_repair =
            existing_ids.is_empty() || matches!(msg.source_mode.as_deref(), None | Some("ai_only"));
        if !needs_repair {
            continue;
        }

        let inferred_ids = infer_cited_page_ids_from_titles(&msg.content, &title_index);
        if inferred_ids.is_empty() {
            continue;
        }

        let source_mode = normalize_source_mode(
            &msg.content,
            msg.source_mode.as_deref().unwrap_or("ai_only"),
            &inferred_ids,
        );
        let pages_json = serde_json::to_string(&inferred_ids)?;
        repo.update_chat_message_sources(&msg.id, &pages_json, &source_mode)?;
        msg.pages_used = Some(pages_json);
        msg.source_mode = Some(source_mode);
    }

    Ok(())
}

/// Stage 0: Rewrite a follow-up question into a standalone query.
async fn rewrite_query(
    db: std::sync::Arc<crate::storage::database::Database>,
    question: &str,
    context: &str,
) -> Result<String, String> {
    let locale = crate::locale::resolve_locale(&db);
    let system = crate::ai::wiki_prompts::query_rewrite_system_prompt(&locale);
    let user = crate::ai::wiki_prompts::query_rewrite_user_message(question, context, &locale);
    let raw = wiki_engine::call_ai_pub(db, &system, &user, 256).await?;
    Ok(raw.trim().to_string())
}

/// Stage 1: Ask AI to pick relevant page IDs from the candidate set.
async fn retrieve_relevant_pages(
    db: std::sync::Arc<crate::storage::database::Database>,
    query: &str,
    context: &str,
    today_iso: &str,
    page_index: &[(String, String, String, String, Option<String>)],
) -> Result<Vec<String>, String> {
    let locale = crate::locale::resolve_locale(&db);
    let system = crate::ai::wiki_prompts::query_retrieve_system_prompt(&locale);
    let user = crate::ai::wiki_prompts::query_retrieve_user_message(
        query, context, today_iso, page_index, &locale,
    );
    let raw = wiki_engine::call_ai_pub(db, &system, &user, 512).await?;
    let json = wiki_engine::parse_ai_json_pub(&raw)?;
    let ids: Vec<String> = json
        .get("page_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(ids)
}

// ===== Chat Session Management =====

#[tauri::command]
pub fn get_chat_sessions(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<WikiChatSession>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_chat_sessions(limit.unwrap_or(20))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_chat_messages(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<WikiChatMessage>, String> {
    let repo = Repository::new(state.db.clone());
    let mut messages = repo
        .get_chat_messages(&session_id)
        .map_err(|e| e.to_string())?;
    repair_chat_message_metadata(&repo, &mut messages).map_err(|e| e.to_string())?;
    Ok(messages)
}

#[tauri::command]
pub fn delete_chat_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    repo.delete_chat_session(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_message_as_page(
    state: State<'_, AppState>,
    session_id: String,
    message_id: String,
) -> Result<WikiPage, String> {
    let repo = Repository::new(state.db.clone());
    let messages = repo
        .get_chat_messages(&session_id)
        .map_err(|e| e.to_string())?;

    let asst_msg = messages
        .iter()
        .find(|m| m.id == message_id && m.role == "assistant")
        .ok_or_else(|| "Message not found".to_string())?;

    // Anti-contamination: only allow saving if source_mode is not ai_only
    let source_mode = asst_msg.source_mode.as_deref().unwrap_or("ai_only");
    if source_mode == "ai_only" {
        return Err(
            "AI-only answers cannot be saved as wiki pages (no knowledge base sources)".to_string(),
        );
    }

    // Dedup: check if this message was already saved (DB-enforced via UNIQUE index)
    if let Ok(Some(existing)) = repo.get_wiki_page_by_source_message_id(&message_id) {
        return Ok(existing);
    }

    // Find the preceding user question
    let user_question = messages
        .iter()
        .rev()
        .find(|m| m.turn_index < asst_msg.turn_index && m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_else(|| "Q&A".to_string());

    let page_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let title: String = user_question.chars().take(40).collect();

    let page = WikiPage {
        id: page_id.clone(),
        title,
        slug: format!("qa-{}", &page_id[..8]),
        page_type: "qa".to_string(),
        body_markdown: format!(
            "## Question\n\n{}\n\n## Answer\n\n{}",
            user_question, asst_msg.content
        ),
        summary: Some(format!(
            "Q&A: {}",
            &user_question.chars().take(30).collect::<String>()
        )),
        tags: None,
        status: "active".to_string(),
        confidence: 0.7,
        created_at: now.clone(),
        updated_at: now.clone(),
        last_compiled_at: Some(now),
        source_message_id: Some(message_id.clone()),
    };

    repo.save_wiki_page(&page).map_err(|e| e.to_string())?;

    // Create deterministic edges from QA page to referenced pages (from pages_used)
    if let Some(ref pages_json) = asst_msg.pages_used {
        for ref_id in crate::storage::repository::parse_page_reference_ids(pages_json) {
            if !ref_id.is_empty() {
                let _ = repo.save_wiki_edge(&page_id, &ref_id, "qa_reference", 1.0);
                let _ = repo.save_wiki_edge(&ref_id, &page_id, "qa_reference", 1.0);
            }
        }
    }

    Ok(page)
}

/// Check which message IDs have already been saved as wiki pages.
#[tauri::command]
pub fn get_saved_message_ids(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let repo = Repository::new(state.db.clone());
    let mut saved = Vec::new();
    for mid in &message_ids {
        if let Ok(Some(_)) = repo.get_wiki_page_by_source_message_id(mid) {
            saved.push(mid.clone());
        }
    }
    Ok(saved)
}

// Legacy compatibility — keep old commands but delegate
#[tauri::command]
pub fn get_wiki_conversations(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<WikiConversation>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_wiki_conversations(limit.unwrap_or(20))
        .map_err(|e| e.to_string())
}

// ===== Tag-based linking =====

// async: a plain command runs on the UI thread, and this rebuilds the links of every page.
#[tauri::command(async)]
pub fn wiki_link_by_tags(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let count = wiki_engine::link_pages_by_shared_tags(db)?;
    Ok(serde_json::json!({ "edges_created": count }))
}

// ===== Lint =====

#[tauri::command]
pub async fn trigger_wiki_lint(state: State<'_, AppState>) -> Result<Vec<WikiLintResult>, String> {
    let repo = Repository::new(state.db.clone());
    run_local_wiki_lint(&repo)
}

fn run_local_wiki_lint(repo: &Repository) -> Result<Vec<WikiLintResult>, String> {
    // Local checks first (no AI needed)
    // Check for needs_recompile pages
    let stale_pages = repo
        .get_wiki_pages_by_status("needs_recompile")
        .map_err(|e| e.to_string())?;
    for page in &stale_pages {
        let page_ids = serde_json::to_string(&[&page.id]).map_err(|e| e.to_string())?;
        repo.save_lint_result(
            "stale",
            "warning",
            &format!("\"{}\" has stale sources", page.title),
            "Some sources have been updated or deleted, recompilation recommended",
            &page_ids,
        )
        .map_err(|e| e.to_string())?;
    }

    // Check for draft (tombstone) pages
    let draft_pages = repo
        .get_wiki_pages_by_status("draft")
        .map_err(|e| e.to_string())?;
    for page in &draft_pages {
        let page_ids = serde_json::to_string(&[&page.id]).map_err(|e| e.to_string())?;
        repo.save_lint_result(
            "orphan",
            "critical",
            &format!("\"{}\" is invalid", page.title),
            "All sources have been deleted, please decide to keep or remove",
            &page_ids,
        )
        .map_err(|e| e.to_string())?;
    }

    let stale_page_ids: HashSet<&str> = stale_pages.iter().map(|page| page.id.as_str()).collect();
    let draft_page_ids: HashSet<&str> = draft_pages.iter().map(|page| page.id.as_str()).collect();
    let results = repo.get_open_lint_results().map_err(|e| e.to_string())?;

    for lint in &results {
        let current_page_ids = match lint.lint_type.as_str() {
            "stale" => Some(&stale_page_ids),
            "orphan" => Some(&draft_page_ids),
            _ => None,
        };
        let Some(current_page_ids) = current_page_ids else {
            continue;
        };
        let page_ids: Vec<String> = serde_json::from_str(&lint.page_ids).unwrap_or_default();
        if !page_ids
            .iter()
            .any(|page_id| current_page_ids.contains(page_id.as_str()))
        {
            repo.resolve_lint_result(lint.id)
                .map_err(|e| e.to_string())?;
        }
    }

    repo.get_open_lint_results().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_wiki_lint_results(state: State<'_, AppState>) -> Result<Vec<WikiLintResult>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_open_lint_results().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn wiki_lint_keep(state: State<'_, AppState>, lint_id: i64) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    // Get the lint result to find affected page
    let lints = repo.get_open_lint_results().map_err(|e| e.to_string())?;
    if let Some(lint) = lints.iter().find(|l| l.id == lint_id) {
        let page_ids: Vec<String> = serde_json::from_str(&lint.page_ids).unwrap_or_default();
        for pid in &page_ids {
            // Restore draft pages to active
            if let Ok(Some(page)) = repo.get_wiki_page_by_id(pid) {
                if page.status == "draft" {
                    let _ = repo.update_wiki_page_status(pid, "active", page.confidence);
                }
            }
        }
    }
    repo.resolve_lint_result(lint_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn wiki_lint_delete(state: State<'_, AppState>, lint_id: i64) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    let lints = repo.get_open_lint_results().map_err(|e| e.to_string())?;
    if let Some(lint) = lints.iter().find(|l| l.id == lint_id) {
        let page_ids: Vec<String> = serde_json::from_str(&lint.page_ids).unwrap_or_default();
        for pid in &page_ids {
            let _ = repo.delete_edges_for_page(pid);
            let _ = repo.delete_sources_for_page(pid);
            let _ = repo.delete_wiki_page(pid);
        }
    }
    repo.resolve_lint_result(lint_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wiki_lint_recompile(
    app: AppHandle,
    state: State<'_, AppState>,
    lint_id: i64,
) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    let lints = repo.get_open_lint_results().map_err(|e| e.to_string())?;
    if let Some(lint) = lints.iter().find(|l| l.id == lint_id) {
        let page_ids: Vec<String> = serde_json::from_str(&lint.page_ids).unwrap_or_default();
        for pid in &page_ids {
            let (active, _) = repo.count_active_sources(pid).map_err(|e| e.to_string())?;
            if active == 0 {
                return Err("No active sources, cannot recompile".to_string());
            }
            // Get active source content IDs and re-compile each
            let sources = repo.get_sources_for_page(pid).map_err(|e| e.to_string())?;
            for src in sources.iter().filter(|s| s.source_status == "active") {
                let _ = wiki_engine::auto_compile(state.db.clone(), &src.content_id).await;
            }
        }
    }
    repo.resolve_lint_result(lint_id)
        .map_err(|e| e.to_string())?;
    let _ = app.emit("wiki-lint-recompile-complete", "done");
    Ok(())
}

// ===== Page Sources (for frontend) =====

#[tauri::command]
pub fn get_page_sources(
    state: State<'_, AppState>,
    page_id: String,
) -> Result<Vec<crate::storage::models::WikiPageSource>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_sources_for_page(&page_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_content_wiki_pages(
    state: State<'_, AppState>,
    content_id: String,
) -> Result<Vec<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    let sources = repo
        .get_pages_for_content(&content_id)
        .map_err(|e| e.to_string())?;
    let mut pages = Vec::new();
    for src in &sources {
        if let Ok(Some(page)) = repo.get_wiki_page_by_id(&src.page_id) {
            if page.status == "active" || page.status == "needs_recompile" {
                pages.push(page);
            }
        }
    }
    Ok(pages)
}

#[tauri::command]
pub fn get_content_wiki_pages_batch(
    state: State<'_, AppState>,
    content_ids: Vec<String>,
) -> Result<std::collections::HashMap<String, Vec<WikiPage>>, String> {
    let repo = Repository::new(state.db.clone());
    let mut result = std::collections::HashMap::new();
    for content_id in content_ids {
        let mut pages = Vec::new();
        for source in repo
            .get_pages_for_content(&content_id)
            .map_err(|e| e.to_string())?
        {
            if let Some(page) = repo
                .get_wiki_page_by_id(&source.page_id)
                .map_err(|e| e.to_string())?
            {
                if page.status == "active" || page.status == "needs_recompile" {
                    pages.push(page);
                }
            }
        }
        result.insert(content_id, pages);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;
    use std::sync::Arc;

    fn make_lint_page(id: &str, status: &str) -> WikiPage {
        WikiPage {
            id: id.to_string(),
            title: format!("Page {id}"),
            slug: format!("page-{id}"),
            page_type: "topic".to_string(),
            body_markdown: "Body".to_string(),
            summary: None,
            tags: None,
            status: status.to_string(),
            confidence: 1.0,
            created_at: "2026-07-12T10:00:00Z".to_string(),
            updated_at: "2026-07-12T10:00:00Z".to_string(),
            last_compiled_at: None,
            source_message_id: None,
        }
    }

    #[test]
    fn lint_scan_deduplicates_persists_and_resolves_disappeared_problems() {
        let db = Arc::new(Database::new_in_memory().unwrap());
        let repo = Repository::new(db.clone());
        repo.save_wiki_page(&make_lint_page("stale-page", "needs_recompile"))
            .unwrap();
        repo.save_wiki_page(&make_lint_page("draft-page", "draft"))
            .unwrap();

        let first = run_local_wiki_lint(&repo).unwrap();
        assert_eq!(first.len(), 2);
        let first_ids: HashSet<i64> = first.iter().map(|lint| lint.id).collect();
        assert_eq!(run_local_wiki_lint(&repo).unwrap().len(), 2);
        let third_ids: HashSet<i64> = run_local_wiki_lint(&repo)
            .unwrap()
            .iter()
            .map(|lint| lint.id)
            .collect();
        assert_eq!(third_ids, first_ids);

        drop(repo);
        let restarted_repo = Repository::new(db);
        let after_restart = restarted_repo.get_open_lint_results().unwrap();
        assert_eq!(after_restart.len(), 2);
        assert!(after_restart.iter().any(|lint| lint.lint_type == "orphan"));

        restarted_repo
            .update_wiki_page_status("stale-page", "active", 1.0)
            .unwrap();
        restarted_repo
            .update_wiki_page_status("draft-page", "active", 1.0)
            .unwrap();
        assert!(run_local_wiki_lint(&restarted_repo).unwrap().is_empty());
    }

    #[test]
    fn strips_inline_uuid_references() {
        let input = "DeepSeek 发布 V4 [5b779fc9-89e9-4981-bdfa-ae5b578fd1c7]，包含 Pro 和 Flash 版";
        let out = strip_inline_page_ids(input);
        assert_eq!(out, "DeepSeek 发布 V4，包含 Pro 和 Flash 版");
    }

    #[test]
    fn strips_multiple_back_to_back_uuids() {
        let input = "[5b779fc9-89e9-4981-bdfa-ae5b578fd1c7] [c6bdc033-f17b-4c99-8eaf-76ec168fada0]";
        let out = strip_inline_page_ids(input);
        assert_eq!(out, "");
    }

    #[test]
    fn preserves_non_uuid_brackets() {
        let input = "[AI 补充] 这是一段补充说明 [RAG 技术]";
        let out = strip_inline_page_ids(input);
        assert_eq!(out, "[AI 补充] 这是一段补充说明 [RAG 技术]");
    }

    #[test]
    fn preserves_uppercase_uuid_too() {
        // Defensive: some models emit uppercase hex
        let input = "页面 [5B779FC9-89E9-4981-BDFA-AE5B578FD1C7] 是核心";
        let out = strip_inline_page_ids(input);
        assert_eq!(out, "页面 是核心");
    }

    #[test]
    fn empty_string_passes_through() {
        assert_eq!(strip_inline_page_ids(""), "");
    }

    // ---- Stage 0 keyword output sanitization ----

    #[test]
    fn sanitize_plain_keywords_passes_through() {
        assert_eq!(
            sanitize_keyword_output("设计 skill"),
            Some("设计 skill".into())
        );
    }

    #[test]
    fn sanitize_strips_markdown_fence() {
        assert_eq!(
            sanitize_keyword_output("```\n设计 skill\n```"),
            Some("设计 skill".into())
        );
    }

    #[test]
    fn sanitize_extracts_query_from_tool_call_format() {
        // The exact failure mode we observed: GLM-style tool-call output
        let raw = r#"[{"name":"ZhipuWebSearch","parameters":{"query":"设计相关的 skill"}}]"#;
        assert_eq!(
            sanitize_keyword_output(raw),
            Some("设计相关的 skill".into())
        );
    }

    #[test]
    fn sanitize_extracts_from_simple_object() {
        let raw = r#"{"query": "RAG 框架"}"#;
        assert_eq!(sanitize_keyword_output(raw), Some("RAG 框架".into()));
    }

    #[test]
    fn sanitize_extracts_from_keywords_field() {
        let raw = r#"{"keywords": "设计 模板"}"#;
        assert_eq!(sanitize_keyword_output(raw), Some("设计 模板".into()));
    }

    #[test]
    fn sanitize_returns_none_for_empty() {
        assert_eq!(sanitize_keyword_output(""), None);
        assert_eq!(sanitize_keyword_output("   "), None);
    }

    #[test]
    fn sanitize_returns_none_for_unparseable_json() {
        // JSON-shaped but not valid → caller falls back to raw question
        assert_eq!(sanitize_keyword_output("{not really json}"), None);
    }

    #[test]
    fn resolves_first_item_follow_up_from_previous_citations() {
        let messages = vec![
            WikiChatMessage {
                id: "u1".to_string(),
                session_id: "s1".to_string(),
                role: "user".to_string(),
                content: "我保存过一个设计的 skill".to_string(),
                pages_used: None,
                source_mode: None,
                turn_index: 0,
                created_at: "2026-04-29T12:26:07Z".to_string(),
            },
            WikiChatMessage {
                id: "a1".to_string(),
                session_id: "s1".to_string(),
                role: "assistant".to_string(),
                content: "候选有好几个".to_string(),
                pages_used: Some("[\"p1\",\"p2\",\"p3\"]".to_string()),
                source_mode: Some("knowledge_base".to_string()),
                turn_index: 1,
                created_at: "2026-04-29T12:26:20Z".to_string(),
            },
            WikiChatMessage {
                id: "u2".to_string(),
                session_id: "s1".to_string(),
                role: "user".to_string(),
                content: "第一个".to_string(),
                pages_used: None,
                source_mode: None,
                turn_index: 2,
                created_at: "2026-04-29T12:26:52Z".to_string(),
            },
        ];

        assert_eq!(
            resolve_follow_up_page_ids("第一个", &messages),
            vec!["p1".to_string()]
        );
    }

    #[test]
    fn infers_cited_pages_from_answer_titles() {
        let answer =
            "看了下，最像你说的是 **awesome-design-md**，另外 `NanoBanana-PPT-Skills` 也沾边。";
        let relevant_pages = vec![(
            "p1".to_string(),
            "awesome-design-md".to_string(),
            "body".to_string(),
        )];
        let page_overview = vec![(
            "p2".to_string(),
            "NanoBanana-PPT-Skills".to_string(),
            "summary".to_string(),
            "2026-04-25T10:00:00Z".to_string(),
            None,
        )];

        let inferred = infer_cited_page_ids(answer, &relevant_pages, &page_overview);
        assert_eq!(inferred.len(), 2);
        assert!(inferred.contains(&"p1".to_string()));
        assert!(inferred.contains(&"p2".to_string()));
    }

    #[test]
    fn upgrades_false_ai_only_when_answer_cites_kb_pages() {
        let answer = "根据候选页面，之前保存的设计相关技能是 **awesome-design-md**。";
        let inferred = vec!["p1".to_string()];
        assert_eq!(
            normalize_source_mode(answer, "ai_only", &inferred),
            "knowledge_base"
        );
    }

    #[test]
    fn prefers_explicitly_cited_titles_over_incidental_mentions() {
        let answer = "**awesome-design-md** 是个开源项目，核心是把 Apple、Stripe 这类顶级网站的设计风格整理出来。";
        let title_index = vec![
            ("p1".to_string(), "awesome-design-md".to_string()),
            ("p2".to_string(), "Apple".to_string()),
        ];

        assert_eq!(
            infer_cited_page_ids_from_titles(answer, &title_index),
            vec!["p1".to_string()]
        );
    }

    #[test]
    fn does_not_infer_generic_short_titles_from_incidental_mentions() {
        let answer = "这类内容和「AI」「设计」都有关系，但这里没有引用具体知识库页面。";
        let title_index = vec![
            ("p1".to_string(), "AI".to_string()),
            ("p2".to_string(), "设计".to_string()),
        ];

        assert!(infer_cited_page_ids_from_titles(answer, &title_index).is_empty());
    }

    #[test]
    fn still_infers_specific_chinese_titles() {
        let answer = "这点可以参考定投策略，里面讲了长期坚持和分散风险。";
        let title_index = vec![("p1".to_string(), "定投策略".to_string())];

        assert_eq!(
            infer_cited_page_ids_from_titles(answer, &title_index),
            vec!["p1".to_string()]
        );
    }
}
