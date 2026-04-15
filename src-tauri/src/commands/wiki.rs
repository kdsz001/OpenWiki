use crate::ai::wiki_engine;
use crate::commands::capture::AppState;
use crate::storage::models::{WikiConversation, WikiLintResult, WikiPage};
use crate::storage::repository::Repository;
use crate::workspace;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use regex::Regex;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

const LOCAL_WIKI_SOURCE_PREFIX: &str = "local-wiki:";
const LOCAL_WIKI_EDGE_RELATION: &str = "local_wiki_link";
const THEME_EDGE_RELATION: &str = "belongs_to_theme";

#[derive(Debug, Clone)]
struct LocalWikiDoc {
    page_type: String,
    title: String,
    slug_seed: String,
    body_markdown: String,
    summary: Option<String>,
    tags: Vec<String>,
    source_message_id: String,
    link_targets: Vec<String>,
}

fn local_wiki_type_from_folder(folder_name: &str) -> Option<&'static str> {
    match folder_name {
        "cases" => Some("case"),
        "concepts" => Some("concept"),
        "themes" => Some("theme"),
        "dashboards" => Some("dashboard"),
        _ => None,
    }
}

fn normalize_local_wiki_key(raw: &str) -> Option<String> {
    let candidate = raw
        .split('|')
        .next()
        .unwrap_or(raw)
        .split('#')
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches('/');
    if candidate.is_empty() {
        return None;
    }
    let candidate = candidate.replace('\\', "/");
    let candidate = candidate
        .strip_suffix(".md")
        .unwrap_or(candidate.as_str())
        .trim()
        .to_string();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_lowercase())
    }
}

fn slugify_local_wiki_title(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_lowercase().next().unwrap_or(c)
            } else if c == ' ' {
                '-'
            } else if ('\u{4E00}'..='\u{9FFF}').contains(&c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    slug.trim_matches('-')
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn truncate_local_text(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn extract_local_wiki_title(markdown: &str, fallback: &str) -> String {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback.to_string()
}

fn extract_local_wiki_summary(markdown: &str) -> Option<String> {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(summary) = trimmed.strip_prefix(">") {
            let summary = summary.trim();
            if !summary.is_empty() {
                return Some(truncate_local_text(summary, 180));
            }
        }
    }

    let mut paragraph_lines = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !paragraph_lines.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#')
            || trimmed.starts_with("```")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("|")
        {
            if !paragraph_lines.is_empty() {
                break;
            }
            continue;
        }
        paragraph_lines.push(trimmed);
    }

    if paragraph_lines.is_empty() {
        None
    } else {
        Some(truncate_local_text(&paragraph_lines.join(" "), 180))
    }
}

fn extract_local_wiki_links(markdown: &str) -> Vec<String> {
    let link_re = Regex::new(r"\[\[([^\]]+)\]\]").expect("local wiki link regex");
    let mut seen = HashSet::new();
    let mut links = Vec::new();

    for captures in link_re.captures_iter(markdown) {
        let Some(raw_target) = captures.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let Some(normalized) = normalize_local_wiki_key(raw_target) else {
            continue;
        };
        if normalized.starts_with("raw/") || normalized.starts_with("raw") {
            continue;
        }
        if seen.insert(normalized.clone()) {
            links.push(normalized);
        }
    }

    links
}

fn render_local_wiki_markdown(markdown: &str) -> String {
    let link_re = Regex::new(r"\[\[([^\]]+)\]\]").expect("local wiki render regex");
    link_re
        .replace_all(markdown, |captures: &regex::Captures| {
            let raw_target = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            let display_text = raw_target
                .split('|')
                .nth(1)
                .or_else(|| raw_target.split('/').last())
                .unwrap_or(raw_target)
                .trim()
                .to_string();
            let Some(normalized_target) = normalize_local_wiki_key(raw_target) else {
                return display_text;
            };
            if normalized_target.starts_with("raw/") {
                return display_text;
            }
            let encoded = URL_SAFE_NO_PAD.encode(normalized_target.as_bytes());
            format!("[{}](openwiki://page/{})", display_text, encoded)
        })
        .into_owned()
}

fn resolve_wiki_link_target(
    repo: &Repository,
    raw_target: &str,
) -> Result<Option<WikiPage>, String> {
    let Some(normalized) = normalize_local_wiki_key(raw_target) else {
        return Ok(None);
    };
    let last_segment = normalized
        .rsplit('/')
        .next()
        .unwrap_or(normalized.as_str())
        .trim()
        .to_string();
    let normalized_spaced = normalized.replace('/', " ");

    let mut slug_candidates = BTreeSet::new();
    for candidate in [&normalized, &last_segment, &normalized_spaced] {
        let slug = slugify_local_wiki_title(candidate);
        if !slug.is_empty() {
            slug_candidates.insert(slug);
        }
    }

    for slug in slug_candidates {
        if let Some(page) = repo
            .get_wiki_page_by_slug(&slug)
            .map_err(|e| format!("Failed to resolve wiki link slug {}: {}", slug, e))?
        {
            return Ok(Some(page));
        }
    }

    for title in [&last_segment, &normalized_spaced, &normalized] {
        if title.trim().is_empty() {
            continue;
        }
        if let Some(page) = repo
            .get_wiki_page_by_title(title)
            .map_err(|e| format!("Failed to resolve wiki link title {}: {}", title, e))?
        {
            return Ok(Some(page));
        }
    }

    Ok(None)
}

fn build_local_wiki_doc(root: &Path, file_path: &Path) -> Result<Option<LocalWikiDoc>, String> {
    let parent_name = file_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Cannot determine page type for {}", file_path.display()))?;
    let Some(page_type) = local_wiki_type_from_folder(parent_name) else {
        return Ok(None);
    };

    let raw_markdown = fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;
    let fallback_title = file_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled");
    let title = extract_local_wiki_title(&raw_markdown, fallback_title);
    let relative_path = file_path.strip_prefix(root).map_err(|e| {
        format!(
            "Failed to resolve relative path for {}: {}",
            file_path.display(),
            e
        )
    })?;
    let relative_no_ext = relative_path.with_extension("");
    let relative_key = relative_no_ext.to_string_lossy().replace('\\', "/");
    let file_stem = file_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(fallback_title);
    let tags = if page_type == "dashboard" {
        vec!["地图".to_string(), "索引".to_string()]
    } else {
        Vec::new()
    };

    Ok(Some(LocalWikiDoc {
        page_type: page_type.to_string(),
        title: title.clone(),
        slug_seed: if relative_key.is_empty() {
            file_stem.to_string()
        } else {
            file_stem.to_string()
        },
        body_markdown: render_local_wiki_markdown(&raw_markdown),
        summary: extract_local_wiki_summary(&raw_markdown),
        tags,
        source_message_id: format!("{}{}", LOCAL_WIKI_SOURCE_PREFIX, file_path.display()),
        link_targets: extract_local_wiki_links(&raw_markdown),
    }))
}

fn ensure_unique_local_slug(
    repo: &Repository,
    slug_seed: &str,
    page_type: &str,
    page_id: &str,
) -> Result<String, String> {
    let base = slugify_local_wiki_title(slug_seed);
    let mut candidate = if base.is_empty() {
        format!("{}-{}", page_type, page_id)
    } else {
        base
    };
    let mut counter = 2usize;
    loop {
        match repo
            .get_wiki_page_by_slug(&candidate)
            .map_err(|e| format!("Failed to inspect slug {}: {}", candidate, e))?
        {
            Some(existing) if existing.id != page_id => {
                candidate = format!("{}-{}", slugify_local_wiki_title(slug_seed), counter);
                counter += 1;
            }
            _ => return Ok(candidate),
        }
    }
}

fn upsert_local_wiki_page(
    repo: &Repository,
    doc: &LocalWikiDoc,
) -> Result<(String, bool, bool), String> {
    let existing_by_source = repo
        .get_wiki_page_by_source_message_id(&doc.source_message_id)
        .map_err(|e| format!("Failed to inspect local wiki page {}: {}", doc.title, e))?;
    let existing_by_slug = repo
        .get_wiki_page_by_slug(&slugify_local_wiki_title(&doc.slug_seed))
        .map_err(|e| format!("Failed to inspect page slug for {}: {}", doc.title, e))?
        .filter(|page| page.page_type == doc.page_type);
    let existing_page = existing_by_source.or(existing_by_slug);
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let tags_json = if doc.tags.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&doc.tags).unwrap_or_default())
    };

    match existing_page {
        Some(existing) => {
            let updated = WikiPage {
                id: existing.id.clone(),
                title: doc.title.clone(),
                slug: ensure_unique_local_slug(repo, &doc.slug_seed, &doc.page_type, &existing.id)?,
                page_type: doc.page_type.clone(),
                body_markdown: doc.body_markdown.clone(),
                summary: doc.summary.clone(),
                tags: tags_json,
                status: "active".to_string(),
                confidence: 1.0,
                created_at: existing.created_at.clone(),
                updated_at: now.clone(),
                last_compiled_at: Some(now),
                source_message_id: Some(doc.source_message_id.clone()),
            };
            repo.update_wiki_page(&updated)
                .map_err(|e| format!("Failed to update local wiki page {}: {}", doc.title, e))?;
            Ok((existing.id, false, true))
        }
        None => {
            let page_id = uuid::Uuid::new_v4().to_string();
            let page = WikiPage {
                id: page_id.clone(),
                title: doc.title.clone(),
                slug: ensure_unique_local_slug(repo, &doc.slug_seed, &doc.page_type, &page_id)?,
                page_type: doc.page_type.clone(),
                body_markdown: doc.body_markdown.clone(),
                summary: doc.summary.clone(),
                tags: tags_json,
                status: "active".to_string(),
                confidence: 1.0,
                created_at: now.clone(),
                updated_at: now.clone(),
                last_compiled_at: Some(now),
                source_message_id: Some(doc.source_message_id.clone()),
            };
            repo.save_wiki_page(&page)
                .map_err(|e| format!("Failed to save local wiki page {}: {}", doc.title, e))?;
            Ok((page_id, true, false))
        }
    }
}

fn resolve_local_wiki_root(path: &Path) -> PathBuf {
    let direct_wiki = path.join("wiki");
    if direct_wiki.is_dir() {
        direct_wiki
    } else {
        path.to_path_buf()
    }
}

#[tauri::command]
pub fn sync_local_wiki(
    state: State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let requested = PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err("Please provide a local knowledge base path".to_string());
    }
    if !requested.exists() {
        return Err(format!("Path does not exist: {}", requested.display()));
    }

    let root = resolve_local_wiki_root(&requested);
    if !root.is_dir() {
        return Err(format!(
            "Local wiki root is not a directory: {}",
            root.display()
        ));
    }

    let repo = Repository::new(state.db.clone());
    let mut docs = Vec::new();

    for folder in ["cases", "concepts", "themes", "dashboards"] {
        let folder_path = root.join(folder);
        if !folder_path.is_dir() {
            continue;
        }
        let read_dir = fs::read_dir(&folder_path)
            .map_err(|e| format!("Failed to read {}: {}", folder_path.display(), e))?;
        for entry in read_dir {
            let entry = entry.map_err(|e| format!("Failed to read local wiki entry: {}", e))?;
            let path = entry.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("md"))
                != Some(true)
            {
                continue;
            }
            if let Some(doc) = build_local_wiki_doc(&root, &path)? {
                docs.push(doc);
            }
        }
    }

    let mut key_to_page_id = HashMap::new();
    let mut source_to_page_id = HashMap::new();
    let mut seen_sources = HashSet::new();
    let mut created = 0usize;
    let mut updated = 0usize;

    for doc in &docs {
        let (page_id, was_created, was_updated) = upsert_local_wiki_page(&repo, doc)?;
        if was_created {
            created += 1;
        }
        if was_updated {
            updated += 1;
        }
        seen_sources.insert(doc.source_message_id.clone());
        source_to_page_id.insert(doc.source_message_id.clone(), page_id.clone());

        let raw_path = doc
            .source_message_id
            .trim_start_matches(LOCAL_WIKI_SOURCE_PREFIX);
        let file_path = Path::new(raw_path);
        let relative_no_ext = file_path
            .strip_prefix(&root)
            .unwrap_or(file_path)
            .with_extension("");
        let relative_key = relative_no_ext.to_string_lossy().replace('\\', "/");
        if let Some(key) = normalize_local_wiki_key(&relative_key) {
            key_to_page_id.insert(key, page_id.clone());
        }
        if let Some(stem) = file_path.file_stem().and_then(|name| name.to_str()) {
            if let Some(key) = normalize_local_wiki_key(stem) {
                key_to_page_id.insert(key, page_id.clone());
            }
        }
        if let Some(key) = normalize_local_wiki_key(&doc.title) {
            key_to_page_id.insert(key, page_id);
        }
    }

    let mut removed = 0usize;
    for page in repo
        .get_all_wiki_pages(10_000, 0)
        .map_err(|e| format!("Failed to inspect existing wiki pages: {}", e))?
    {
        let Some(source_message_id) = page.source_message_id.as_deref() else {
            continue;
        };
        if !source_message_id.starts_with(LOCAL_WIKI_SOURCE_PREFIX)
            || seen_sources.contains(source_message_id)
        {
            continue;
        }
        repo.delete_wiki_page_fully(&page.id).map_err(|e| {
            format!(
                "Failed to remove stale local wiki page {}: {}",
                page.title, e
            )
        })?;
        removed += 1;
    }

    let _ = repo.delete_edges_by_relation(LOCAL_WIKI_EDGE_RELATION);
    let mut edges_created = 0usize;
    for doc in &docs {
        let Some(from_page_id) = source_to_page_id.get(&doc.source_message_id) else {
            continue;
        };
        let mut linked_targets = HashSet::new();
        for target in &doc.link_targets {
            let Some(target_page_id) = key_to_page_id.get(target) else {
                continue;
            };
            if target_page_id == from_page_id || !linked_targets.insert(target_page_id.clone()) {
                continue;
            }
            repo.save_wiki_edge(from_page_id, target_page_id, LOCAL_WIKI_EDGE_RELATION, 1.0)
                .map_err(|e| format!("Failed to save local wiki edge for {}: {}", doc.title, e))?;
            edges_created += 1;
        }
    }

    let counts_by_type = repo
        .get_all_wiki_pages(10_000, 0)
        .map_err(|e| format!("Failed to refresh wiki page counts: {}", e))?
        .into_iter()
        .fold(HashMap::<String, usize>::new(), |mut acc, page| {
            *acc.entry(page.page_type).or_insert(0) += 1;
            acc
        });

    Ok(serde_json::json!({
        "root": root.display().to_string(),
        "pages_found": docs.len(),
        "created": created,
        "updated": updated,
        "removed": removed,
        "edges_created": edges_created,
        "counts": counts_by_type,
    }))
}

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
        let fetch_limit = lim.max(500);
        repo.get_all_wiki_pages(fetch_limit, off)
            .map(|pages| {
                pages
                    .into_iter()
                    .filter(|page| page.page_type != "source" && page.page_type != "qa")
                    .take(lim as usize)
                    .collect()
            })
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_wiki_page(state: State<'_, AppState>, id: String) -> Result<Option<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_wiki_page_by_id(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolve_wiki_link(
    state: State<'_, AppState>,
    target: String,
) -> Result<Option<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    resolve_wiki_link_target(&repo, &target)
}

#[tauri::command]
pub fn search_wiki(state: State<'_, AppState>, query: String) -> Result<Vec<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    repo.search_wiki_pages(&query, 20)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_page_themes(
    state: State<'_, AppState>,
    page_id: String,
) -> Result<Vec<WikiPage>, String> {
    let repo = Repository::new(state.db.clone());
    repo.get_target_pages_by_relation(&page_id, THEME_EDGE_RELATION)
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
    let pages = repo
        .get_all_wiki_pages(2_000, 0)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|page| page.page_type != "source" && page.page_type != "qa")
        .take(800)
        .collect::<Vec<_>>();
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
            let _ = wiki_engine::demote_low_support_concepts(db.clone());
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

#[tauri::command]
pub async fn compile_contents_to_wiki(
    app: AppHandle,
    state: State<'_, AppState>,
    content_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let repo = Repository::new(db.clone());
    let _ = repo.cleanup_stale_compile_locks_older_than(300);
    let mut compiled = 0usize;
    let mut failed = 0usize;
    let mut touched_pages = BTreeSet::new();

    let _ = app.emit(
        "wiki-compile-progress",
        serde_json::json!({
            "status": "compiling",
            "current": 0,
            "total": content_ids.len(),
            "compiled": 0,
            "failed": 0,
        }),
    );

    for (index, content_id) in content_ids.iter().enumerate() {
        match wiki_engine::manual_compile(db.clone(), content_id).await {
            Ok(touched_ids) => {
                compiled += 1;
                for page_id in touched_ids {
                    touched_pages.insert(page_id);
                }
            }
            Err(e) => {
                failed += 1;
                log::warn!("Wiki batch compile error for {}: {}", content_id, e);
            }
        }

        let _ = app.emit(
            "wiki-compile-progress",
            serde_json::json!({
                "status": "compiling",
                "current": index + 1,
                "total": content_ids.len(),
                "compiled": compiled,
                "failed": failed,
                "content_id": content_id,
            }),
        );
    }

    let _ = wiki_engine::demote_low_support_concepts(db.clone());
    let _ = wiki_engine::link_pages_by_shared_tags(db);

    let page_ids: Vec<String> = touched_pages.into_iter().collect();
    let _ = app.emit("wiki-compile-complete", &page_ids);

    Ok(serde_json::json!({
        "processed": content_ids.len(),
        "compiled": compiled,
        "failed": failed,
        "touched_pages": page_ids.len(),
        "page_ids": page_ids,
    }))
}

#[tauri::command]
pub async fn trigger_wiki_auto_compile(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let repo = Repository::new(db.clone());
    let _ = repo.cleanup_stale_compile_locks_older_than(300);

    let total_items: i64 = state
        .db
        .conn
        .lock()
        .map_err(|e| e.to_string())?
        .query_row(
            "SELECT COUNT(*) FROM captured_content WHERE is_deleted = 0",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    // Find content across the whole library so imported notes are included too
    let all_content = repo
        .get_all_content(total_items, 0)
        .map_err(|e| e.to_string())?;

    let mut pending_content = Vec::new();
    let mut already_up_to_date = 0usize;

    for content in all_content {
        let is_imported_file = Path::new(&content.source_app)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt"))
            .unwrap_or(false);
        let current_hash = wiki_engine::compute_content_hash_with_db(db.clone(), &content);
        let already_processed = content.wiki_assessed_hash.as_deref() == Some(&current_hash)
            || content.wiki_compile_hash.as_deref() == Some(&current_hash);

        if already_processed {
            let needs_backfill = if is_imported_file {
                wiki_engine::content_needs_structured_backfill(&repo, &content).unwrap_or(true)
            } else {
                false
            };
            if !needs_backfill {
                already_up_to_date += 1;
                continue;
            }
        }

        pending_content.push((content, is_imported_file));
    }

    let mut compiled = 0;
    let skipped = 0;
    let mut errors = 0;

    let _ = app.emit(
        "wiki-auto-compile-progress",
        serde_json::json!({
            "status": "compiling",
            "current": 0,
            "total": pending_content.len(),
            "compiled": 0,
            "skipped": 0,
            "errors": 0,
            "up_to_date": already_up_to_date,
        }),
    );

    for (index, (content, is_imported_file)) in pending_content.iter().enumerate() {
        let result = if *is_imported_file {
            wiki_engine::manual_compile(db.clone(), &content.id)
                .await
                .map(|_| ())
        } else {
            wiki_engine::auto_compile(db.clone(), &content.id).await
        };

        match result {
            Ok(()) => compiled += 1,
            Err(e) => {
                log::warn!("Wiki auto-compile error for {}: {}", content.id, e);
                errors += 1;
            }
        }

        let _ = app.emit(
            "wiki-auto-compile-progress",
            serde_json::json!({
                "status": "compiling",
                "current": index + 1,
                "total": pending_content.len(),
                "compiled": compiled,
                "skipped": skipped,
                "errors": errors,
                "content_id": content.id,
                "up_to_date": already_up_to_date,
            }),
        );
    }

    let demoted = wiki_engine::demote_low_support_concepts(db.clone()).unwrap_or(0);
    // Auto-link pages by shared tags after batch compilation
    let tag_edges = wiki_engine::link_pages_by_shared_tags(db).unwrap_or(0);

    let _ = app.emit("wiki-auto-compile-complete", "done");

    Ok(serde_json::json!({
        "processed": compiled + skipped,
        "compiled": compiled,
        "errors": errors,
        "demoted_concepts": demoted,
        "tag_edges": tag_edges,
        "up_to_date": already_up_to_date,
        "remaining": pending_content.len().saturating_sub(compiled + skipped + errors),
    }))
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

    // Build conversation context from recent turns
    let messages = repo
        .get_chat_messages(&session_id)
        .map_err(|e| e.to_string())?;
    let recent_context = build_conversation_context(&messages, 3);

    // Stage 0: Query rewrite (if multi-turn)
    let search_query = if messages.len() > 1 {
        match rewrite_query(db.clone(), &question, &recent_context).await {
            Ok(q) => q,
            Err(_) => question.clone(), // fallback to original
        }
    } else {
        question.clone()
    };

    // Stage 1: Retrieve relevant page IDs via AI
    let page_index = repo
        .get_wiki_page_summaries_for_qa()
        .map_err(|e| e.to_string())?;
    let relevant_ids = if page_index.is_empty() {
        vec![]
    } else {
        match retrieve_relevant_pages(db.clone(), &search_query, &recent_context, &page_index).await
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
        &question,
        &recent_context,
        &relevant_pages,
        &page_index,
        &locale,
    );

    let raw = wiki_engine::call_ai_pub(db.clone(), &answer_system, &answer_user, 2048).await?;

    // Parse response — graceful fallback
    let (answer, page_ids_used, source_mode, confidence) =
        match wiki_engine::parse_ai_json_pub(&raw) {
            Ok(json) => {
                let a = json
                    .get("answer")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&raw)
                    .to_string();
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
                (a, pids, sm, c)
            }
            Err(_) => {
                // Malformed JSON — try to extract "answer" field via regex
                let extracted = extract_answer_from_malformed_json(&raw);
                (extracted, vec![], "ai_only".to_string(), 0.3)
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

/// Build conversation context string from recent messages (last N turns).
fn build_conversation_context(messages: &[WikiChatMessage], max_turns: usize) -> String {
    let recent: Vec<&WikiChatMessage> = messages.iter().rev().take(max_turns * 2).collect();
    let mut parts = Vec::new();
    let mut budget = 2000i64;
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

/// Stage 1: Ask AI to pick relevant page IDs from the index.
async fn retrieve_relevant_pages(
    db: std::sync::Arc<crate::storage::database::Database>,
    query: &str,
    context: &str,
    page_index: &[(String, String, String)],
) -> Result<Vec<String>, String> {
    let locale = crate::locale::resolve_locale(&db);
    let system = crate::ai::wiki_prompts::query_retrieve_system_prompt(&locale);
    let user =
        crate::ai::wiki_prompts::query_retrieve_user_message(query, context, page_index, &locale);
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
    repo.get_chat_messages(&session_id)
        .map_err(|e| e.to_string())
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
    let _ = workspace::sync_wiki_page_to_workspace_draft(&repo, &page);

    // Create deterministic edges from QA page to referenced pages (from pages_used)
    if let Some(ref pages_json) = asst_msg.pages_used {
        let referenced_ids: Vec<String> = serde_json::from_str(pages_json).unwrap_or_default();
        for ref_item in &referenced_ids {
            // pages_used may contain {id, title} objects or plain strings
            let ref_id = if let Ok(obj) = serde_json::from_str::<serde_json::Value>(ref_item) {
                obj.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(ref_item)
                    .to_string()
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
pub fn wiki_link_by_tags(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let count = wiki_engine::link_pages_by_shared_tags(db)?;
    Ok(serde_json::json!({ "edges_created": count }))
}

// ===== Lint =====

#[tauri::command]
pub async fn trigger_wiki_lint(state: State<'_, AppState>) -> Result<Vec<WikiLintResult>, String> {
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

    results = repo.get_open_lint_results().map_err(|e| e.to_string())?;

    Ok(results)
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
