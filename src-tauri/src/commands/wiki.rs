use crate::ai::wiki_engine;
use crate::commands::capture::AppState;
use crate::storage::models::{WikiConversation, WikiLintResult, WikiPage};
use crate::storage::repository::Repository;
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
        repo.get_all_wiki_pages(lim, off)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_wiki_page(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_wiki_page_by_id(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_wiki(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<WikiPage>, String> {
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
pub fn delete_wiki_page(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
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
    let pages = repo
        .get_all_wiki_pages(500, 0)
        .map_err(|e| e.to_string())?;
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

use crate::storage::models::{WikiChatSession, WikiChatMessage};

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
    let user_turn = repo.get_next_turn_index(&session_id).map_err(|e| e.to_string())?;
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
    repo.add_chat_message(&user_msg).map_err(|e| e.to_string())?;

    // Build conversation context from recent turns. The real cap is the
    // character budget inside `build_conversation_context` — the turn
    // count is just a safety net so we don't walk a 5000-row table for
    // a chat that's been alive for months. Modern LLMs handle 200k+
    // context, so being generous here costs essentially nothing.
    let messages = repo.get_chat_messages(&session_id).map_err(|e| e.to_string())?;
    let recent_context = build_conversation_context(&messages, 50);

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
            db.clone(), &search_query, &recent_context, &today_iso, &page_index,
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
        &question, &recent_context, &today_iso, &relevant_pages, &page_index, &locale,
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
                let pids: Vec<String> = json.get("page_ids_used")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let sm = json.get("source_mode").and_then(|v| v.as_str()).unwrap_or(
                    if pids.is_empty() { "ai_only" } else { "knowledge_base" }
                ).to_string();
                let c = json.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
                (a, pids, sm, c)
            }
            Err(_) => {
                // Malformed JSON — try to extract "answer" field via regex
                let extracted = strip_inline_page_ids(&extract_answer_from_malformed_json(&raw));
                (extracted, vec![], "ai_only".to_string(), 0.3)
            }
        };

    // Save assistant message
    let asst_turn = repo.get_next_turn_index(&session_id).map_err(|e| e.to_string())?;
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
    repo.add_chat_message(&asst_msg).map_err(|e| e.to_string())?;
    let _ = repo.touch_chat_session(&session_id);

    // Resolve page titles for frontend display
    let page_titles: Vec<serde_json::Value> = page_ids_used.iter().filter_map(|id| {
        repo.get_wiki_page_by_id(id).ok().flatten().map(|p| {
            serde_json::json!({"id": p.id, "title": p.title})
        })
    }).collect();

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
                            other => { result.push('\\'); result.push(other); }
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
        let role_label = if msg.role == "user" { "User" } else { "Assistant" };
        let content: String = msg.content.chars().take(budget.max(0) as usize).collect();
        budget -= content.len() as i64;
        parts.push(format!("{}: {}", role_label, content));
        if budget <= 0 { break; }
    }
    parts.join("\n")
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
    let ids: Vec<String> = json.get("page_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
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
    repo.get_chat_messages(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_chat_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
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
    let messages = repo.get_chat_messages(&session_id).map_err(|e| e.to_string())?;

    let asst_msg = messages.iter().find(|m| m.id == message_id && m.role == "assistant")
        .ok_or_else(|| "Message not found".to_string())?;

    // Anti-contamination: only allow saving if source_mode is not ai_only
    let source_mode = asst_msg.source_mode.as_deref().unwrap_or("ai_only");
    if source_mode == "ai_only" {
        return Err("AI-only answers cannot be saved as wiki pages (no knowledge base sources)".to_string());
    }

    // Dedup: check if this message was already saved (DB-enforced via UNIQUE index)
    if let Ok(Some(existing)) = repo.get_wiki_page_by_source_message_id(&message_id) {
        return Ok(existing);
    }

    // Find the preceding user question
    let user_question = messages.iter().rev()
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
        body_markdown: format!("## Question\n\n{}\n\n## Answer\n\n{}", user_question, asst_msg.content),
        summary: Some(format!("Q&A: {}", &user_question.chars().take(30).collect::<String>())),
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
        let referenced_ids: Vec<String> = serde_json::from_str(pages_json).unwrap_or_default();
        for ref_item in &referenced_ids {
            // pages_used may contain {id, title} objects or plain strings
            let ref_id = if let Ok(obj) = serde_json::from_str::<serde_json::Value>(ref_item) {
                obj.get("id").and_then(|v| v.as_str()).unwrap_or(ref_item).to_string()
            } else {
                ref_item.clone()
            };
            if !ref_id.is_empty() {
                let _ = repo.save_wiki_edge(&page_id, &ref_id, "related", 1.0);
                let _ = repo.save_wiki_edge(&ref_id, &page_id, "related", 1.0); // bidirectional
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

#[tauri::command]
pub fn wiki_link_by_tags(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let count = wiki_engine::link_pages_by_shared_tags(db)?;
    Ok(serde_json::json!({ "edges_created": count }))
}

// ===== Lint =====

#[tauri::command]
pub async fn trigger_wiki_lint(
    state: State<'_, AppState>,
) -> Result<Vec<WikiLintResult>, String> {
    let repo = Repository::new(state.db.clone());

    // Local checks first (no AI needed)
    let mut results = Vec::new();

    // Check for needs_recompile pages
    let stale_pages = repo
        .get_wiki_pages_by_status("needs_recompile")
        .map_err(|e| e.to_string())?;
    for page in &stale_pages {
        let _ = repo.save_lint_result(
            "stale",
            "warning",
            &format!("\"{}\" has stale sources", page.title),
            "Some sources have been updated or deleted, recompilation recommended",
            &format!("[\"{}\"]", page.id),
        );
    }

    // Check for draft (tombstone) pages
    let draft_pages = repo
        .get_wiki_pages_by_status("draft")
        .map_err(|e| e.to_string())?;
    for page in &draft_pages {
        let _ = repo.save_lint_result(
            "orphan",
            "critical",
            &format!("\"{}\" is invalid", page.title),
            "All sources have been deleted, please decide to keep or remove",
            &format!("[\"{}\"]", page.id),
        );
    }

    results = repo
        .get_open_lint_results()
        .map_err(|e| e.to_string())?;

    Ok(results)
}

#[tauri::command]
pub fn get_wiki_lint_results(
    state: State<'_, AppState>,
) -> Result<Vec<WikiLintResult>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_open_lint_results().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn wiki_lint_keep(
    state: State<'_, AppState>,
    lint_id: i64,
) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    // Get the lint result to find affected page
    let lints = repo.get_open_lint_results().map_err(|e| e.to_string())?;
    if let Some(lint) = lints.iter().find(|l| l.id == lint_id) {
        let page_ids: Vec<String> =
            serde_json::from_str(&lint.page_ids).unwrap_or_default();
        for pid in &page_ids {
            // Restore draft pages to active
            if let Ok(Some(page)) = repo.get_wiki_page_by_id(pid) {
                if page.status == "draft" {
                    let _ = repo.update_wiki_page_status(pid, "active", page.confidence);
                }
            }
        }
    }
    repo.resolve_lint_result(lint_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn wiki_lint_delete(
    state: State<'_, AppState>,
    lint_id: i64,
) -> Result<(), String> {
    let repo = Repository::new(state.db.clone());
    let lints = repo.get_open_lint_results().map_err(|e| e.to_string())?;
    if let Some(lint) = lints.iter().find(|l| l.id == lint_id) {
        let page_ids: Vec<String> =
            serde_json::from_str(&lint.page_ids).unwrap_or_default();
        for pid in &page_ids {
            let _ = repo.delete_edges_for_page(pid);
            let _ = repo.delete_sources_for_page(pid);
            let _ = repo.delete_wiki_page(pid);
        }
    }
    repo.resolve_lint_result(lint_id)
        .map_err(|e| e.to_string())
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
        let page_ids: Vec<String> =
            serde_json::from_str(&lint.page_ids).unwrap_or_default();
        for pid in &page_ids {
            let (active, _) = repo
                .count_active_sources(pid)
                .map_err(|e| e.to_string())?;
            if active == 0 {
                return Err("No active sources, cannot recompile".to_string());
            }
            // Get active source content IDs and re-compile each
            let sources = repo
                .get_sources_for_page(pid)
                .map_err(|e| e.to_string())?;
            for src in sources.iter().filter(|s| s.source_status == "active") {
                let _ =
                    wiki_engine::auto_compile(state.db.clone(), &src.content_id).await;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(sanitize_keyword_output("设计 skill"), Some("设计 skill".into()));
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
}
