use crate::storage::models::{CapturedContent, UserPreference};
use std::collections::HashSet;

/// Minimum character count for text content to be included in the weekly report.
const MIN_TEXT_LENGTH: usize = 10;

/// Maximum number of content items to send to the AI.
/// Prevents oversized prompts when users save a lot of content.
const MAX_ITEMS_FOR_AI: usize = 50;

/// Similarity threshold for n-gram deduplication (0.0–1.0).
/// Items with similarity above this are considered near-duplicates.
const SIMILARITY_THRESHOLD: f64 = 0.6;

/// N-gram size for similarity comparison.
const NGRAM_SIZE: usize = 3;

/// A scored content item ready for ranking.
#[derive(Debug)]
pub struct ScoredContent<'a> {
    pub item: &'a CapturedContent,
    pub importance: f64,
}

/// Smart pre-filtering pipeline for weekly report generation.
///
/// Steps:
/// 1. Basic junk filtering (short text, code, paths)
/// 2. Importance scoring (text richness, user notes, type weight, preference match)
/// 3. Similarity-based deduplication (n-gram Jaccard similarity)
/// 4. Top-N selection with category balancing
///
/// Returns (scored_items, filtered_count).
pub fn smart_filter_for_report<'a>(
    contents: &'a [CapturedContent],
    preferences: &[UserPreference],
) -> (Vec<ScoredContent<'a>>, usize) {
    // Step 1: Basic junk filtering (same as before)
    let (clean_items, filtered_count) = filter_for_report(contents);

    if clean_items.is_empty() {
        return (Vec::new(), filtered_count);
    }

    // Step 2: Score each item by importance
    let preference_topics: Vec<(&str, f64)> = preferences
        .iter()
        .filter(|p| p.weight > 0.0)
        .map(|p| (p.topic.as_str(), p.weight))
        .collect();

    let mut scored: Vec<ScoredContent<'_>> = clean_items
        .into_iter()
        .map(|item| {
            let importance = compute_importance(item, &preference_topics);
            ScoredContent { item, importance }
        })
        .collect();

    // Sort by importance descending
    scored.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Step 3: Similarity-based deduplication
    let deduped = deduplicate_similar(scored);
    let dedup_removed = clean_items_count_before_dedup(&deduped, contents.len(), filtered_count);

    // Step 4: Top-N with category balancing
    let final_items = balance_and_cap(deduped, MAX_ITEMS_FOR_AI);

    let total_filtered = contents.len() - final_items.len();

    log::info!(
        "智能预筛: 总计 {} 条 → 基础过滤去除 {} 条 → 相似去重 {} 条 → 最终保留 {} 条",
        contents.len(),
        filtered_count,
        dedup_removed,
        final_items.len()
    );

    (final_items, total_filtered)
}

/// Compute importance score for a content item (0.0–1.0 scale).
fn compute_importance(item: &CapturedContent, preference_topics: &[(&str, f64)]) -> f64 {
    let mut score: f64 = 0.0;

    let content_type = item.content_type.as_str();
    let text = item.raw_text.as_deref().unwrap_or("");
    let char_count = text.chars().count();

    // --- Factor 1: Content type weight (0–0.2) ---
    // URL articles are typically most substantive, then text, then images
    let type_weight = match content_type {
        "url" => 0.20,
        "text" => 0.12,
        "image" => 0.08,
        _ => 0.10,
    };
    score += type_weight;

    // --- Factor 2: Text richness (0–0.25) ---
    // Longer, more substantive text gets higher score
    let richness = if char_count == 0 {
        0.0
    } else if char_count < 30 {
        0.05
    } else if char_count < 100 {
        0.10
    } else if char_count < 300 {
        0.15
    } else if char_count < 800 {
        0.20
    } else {
        0.25
    };
    score += richness;

    // --- Factor 3: User note presence (0 or 0.20) ---
    // If user added a note, they explicitly cared about this content
    if let Some(note) = &item.user_note {
        if !note.trim().is_empty() {
            score += 0.20;
        }
    }

    // --- Factor 4: URL with fetched content bonus (0 or 0.10) ---
    // URL content where we successfully fetched the article body
    if content_type == "url" {
        if let (Some(url), Some(text)) = (&item.source_url, &item.raw_text) {
            // If raw_text differs from the URL itself, we have fetched content
            if text != url && char_count > 50 {
                score += 0.10;
            }
        }
    }

    // --- Factor 5: Preference match boost (0–0.25) ---
    // Boost items that match user's known interests
    if !preference_topics.is_empty() && !text.is_empty() {
        let text_lower = text.to_lowercase();
        let mut pref_boost: f64 = 0.0;
        for (topic, weight) in preference_topics {
            if text_lower.contains(&topic.to_lowercase()) {
                // Scale boost by preference weight, capped contribution per topic
                pref_boost += (weight * 0.05).min(0.10);
            }
        }
        score += pref_boost.min(0.25);
    }

    score.min(1.0)
}

/// Deduplicate items that are too similar using n-gram Jaccard similarity.
/// Keeps the higher-scored item when two items are similar.
/// Input must be sorted by importance (descending).
fn deduplicate_similar(items: Vec<ScoredContent<'_>>) -> Vec<ScoredContent<'_>> {
    let mut kept: Vec<ScoredContent<'_>> = Vec::with_capacity(items.len());
    let mut kept_ngrams: Vec<HashSet<String>> = Vec::new();

    for scored in items {
        let text = scored.item.raw_text.as_deref().unwrap_or("");

        // Images without text — always keep (can't compare)
        if text.is_empty() {
            kept.push(scored);
            continue;
        }

        let ngrams = extract_ngrams(text, NGRAM_SIZE);

        // Check similarity against all kept items
        let is_similar = kept_ngrams
            .iter()
            .any(|existing| jaccard_similarity(&ngrams, existing) > SIMILARITY_THRESHOLD);

        if !is_similar {
            kept_ngrams.push(ngrams);
            kept.push(scored);
        } else {
            log::debug!("去重相似内容: {}", &text[..text.len().min(50)]);
        }
    }

    kept
}

/// Balance content types and cap to max_items.
/// Ensures each type gets fair representation:
/// - At least 20% of slots for each present type (if available)
/// - Remaining slots filled by highest importance regardless of type
fn balance_and_cap(items: Vec<ScoredContent<'_>>, max_items: usize) -> Vec<ScoredContent<'_>> {
    if items.len() <= max_items {
        return items;
    }

    // Group by content type
    let mut by_type: std::collections::HashMap<&str, Vec<ScoredContent<'_>>> =
        std::collections::HashMap::new();
    for scored in items {
        by_type
            .entry(scored.item.content_type.as_str())
            .or_default()
            .push(scored);
    }

    let type_count = by_type.len();
    // Each type gets at least 20% of max_items (if they have enough items)
    let min_per_type = max_items / type_count.max(1) / 2;

    let mut result: Vec<ScoredContent<'_>> = Vec::with_capacity(max_items);
    let mut overflow: Vec<ScoredContent<'_>> = Vec::new();

    for (_type_name, type_items) in by_type.iter_mut() {
        // Items are already sorted by importance within the full list
        // Re-sort just in case
        type_items.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Take guaranteed slots per type
    for (_type_name, type_items) in &mut by_type {
        let take = min_per_type.min(type_items.len());
        result.extend(type_items.drain(..take));
    }

    // Collect remaining items and sort by importance
    for (_, remaining) in by_type {
        overflow.extend(remaining);
    }
    overflow.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Fill remaining slots with highest importance items
    let remaining_slots = max_items.saturating_sub(result.len());
    result.extend(overflow.into_iter().take(remaining_slots));

    // Final sort by importance
    result.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    result
}

/// Extract character n-grams from text for similarity comparison.
fn extract_ngrams(text: &str, n: usize) -> HashSet<String> {
    let chars: Vec<char> = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    if chars.len() < n {
        let mut set = HashSet::new();
        set.insert(chars.iter().collect());
        return set;
    }

    chars.windows(n).map(|w| w.iter().collect()).collect()
}

/// Jaccard similarity between two sets of n-grams (0.0–1.0).
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Helper to compute how many items were removed during dedup step.
fn clean_items_count_before_dedup(
    deduped: &[ScoredContent<'_>],
    total: usize,
    basic_filtered: usize,
) -> usize {
    let after_basic = total - basic_filtered;
    after_basic.saturating_sub(deduped.len())
}

// ============================================================
// Original basic filter (still used as step 1 of smart pipeline)
// ============================================================

/// Filter out junk/noise content before sending to the AI for weekly report generation.
/// Returns (kept, filtered_count) where `kept` is the list of meaningful content items.
pub fn filter_for_report(contents: &[CapturedContent]) -> (Vec<&CapturedContent>, usize) {
    let mut kept: Vec<&CapturedContent> = Vec::new();
    let mut seen_texts: HashSet<String> = HashSet::new();
    let mut filtered = 0usize;

    for item in contents {
        let dominated_type = item.content_type.as_str();

        // Rule 1: Always keep URL content (fetched articles have value)
        if dominated_type == "url" {
            kept.push(item);
            continue;
        }

        // Rule 2: Always keep images (they were intentionally captured)
        if dominated_type == "image" {
            kept.push(item);
            continue;
        }

        // For text content, apply filtering rules
        let raw = match &item.raw_text {
            Some(t) if !t.is_empty() => t,
            _ => {
                filtered += 1;
                continue;
            }
        };

        let trimmed = raw.trim();

        // Rule 3: Filter out text that is too short (< 10 chars)
        if trimmed.chars().count() < MIN_TEXT_LENGTH {
            log::debug!("过滤短文本 ({}字): {}", trimmed.chars().count(), trimmed);
            filtered += 1;
            continue;
        }

        // Rule 4: Filter out code, file paths, and shell commands
        if looks_like_code_or_path(trimmed) {
            log::debug!("过滤代码/路径: {}", &trimmed[..trimmed.len().min(60)]);
            filtered += 1;
            continue;
        }

        // Rule 5: Filter out near-duplicate text (normalize and check first 64 chars)
        let normalized = normalize_for_dedup(trimmed);
        if seen_texts.contains(&normalized) {
            log::debug!("过滤重复内容: {}", &trimmed[..trimmed.len().min(40)]);
            filtered += 1;
            continue;
        }
        seen_texts.insert(normalized);

        kept.push(item);
    }

    (kept, filtered)
}

/// Heuristic: does this text look like code, a file path, or a shell command?
fn looks_like_code_or_path(text: &str) -> bool {
    let trimmed = text.trim();

    // File paths (Unix or Windows)
    if trimmed.starts_with('/')
        && trimmed.contains('/')
        && !trimmed.contains(' ')
        && trimmed.len() < 300
    {
        return true;
    }
    if trimmed.contains(":\\") || trimmed.contains("C:/") {
        return true;
    }

    // Shell commands: common prefixes
    let cmd_prefixes = [
        "cd ", "ls ", "rm ", "cp ", "mv ", "mkdir ", "chmod ", "chown ", "sudo ", "npm ", "npx ",
        "yarn ", "pnpm ", "cargo ", "git ", "docker ", "brew ", "pip ", "python ", "node ",
        "curl ", "wget ", "ssh ", "scp ",
    ];
    let lower = trimmed.to_lowercase();
    for prefix in &cmd_prefixes {
        if lower.starts_with(prefix) && !trimmed.contains('\n') {
            return true;
        }
    }

    // Code patterns: high density of code-specific characters
    let code_chars = ['{', '}', '(', ')', ';', '=', '<', '>', '|', '&'];
    let total_chars = trimmed.chars().count();
    if total_chars > 0 {
        let code_char_count = trimmed.chars().filter(|c| code_chars.contains(c)).count();
        let ratio = code_char_count as f64 / total_chars as f64;
        // If more than 15% of characters are code-specific, likely code
        if ratio > 0.15 && total_chars < 500 {
            return true;
        }
    }

    // Import / require statements
    if trimmed.starts_with("import ") || trimmed.starts_with("from ") && trimmed.contains("import")
    {
        return true;
    }
    if trimmed.starts_with("const ") || trimmed.starts_with("let ") || trimmed.starts_with("var ") {
        if trimmed.contains('=') {
            return true;
        }
    }
    if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") || trimmed.starts_with("def ") {
        return true;
    }

    false
}

/// Normalize text for deduplication: lowercase, collapse whitespace, take first 64 chars.
fn normalize_for_dedup(text: &str) -> String {
    let lower = text.to_lowercase();
    let collapsed: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_text_filtered() {
        assert!(looks_like_code_or_path("/usr/local/bin/foo") == true);
    }

    #[test]
    fn test_normal_text_kept() {
        assert!(looks_like_code_or_path("今天学到了一个很有趣的知识") == false);
    }

    #[test]
    fn test_shell_command_filtered() {
        assert!(looks_like_code_or_path("npm install react") == true);
        assert!(looks_like_code_or_path("git commit -m 'test'") == true);
    }

    #[test]
    fn test_code_filtered() {
        assert!(looks_like_code_or_path("const x = { a: 1, b: 2 };") == true);
        assert!(looks_like_code_or_path("import React from 'react'") == true);
    }

    #[test]
    fn test_jaccard_similarity() {
        let a = extract_ngrams("今天天气很好啊", 3);
        let b = extract_ngrams("今天天气很好呢", 3);
        let sim = jaccard_similarity(&a, &b);
        assert!(
            sim > 0.5,
            "Similar texts should have high Jaccard similarity: {}",
            sim
        );

        let c = extract_ngrams("完全不同的内容", 3);
        let sim2 = jaccard_similarity(&a, &c);
        assert!(
            sim2 < 0.3,
            "Different texts should have low similarity: {}",
            sim2
        );
    }

    #[test]
    fn test_importance_scoring() {
        use crate::storage::models::ContentType;

        let make_item = |content_type: &str, text: &str, note: Option<&str>| -> CapturedContent {
            CapturedContent {
                id: "test".to_string(),
                content_type: ContentType::from_str(content_type),
                raw_text: Some(text.to_string()),
                image_path: None,
                thumbnail_path: None,
                source_app: "test".to_string(),
                source_bundle_id: None,
                source_url: None,
                user_note: note.map(|s| s.to_string()),
                captured_at: "2025-01-01T00:00:00Z".to_string(),
                content_hash: "hash".to_string(),
                byte_size: 0,
                is_deleted: false,
                created_at: "2025-01-01T00:00:00Z".to_string(),
                updated_at: "2025-01-01T00:00:00Z".to_string(),
                digested_at: None,
                digest_action: None,
                summary: None,
                tags: None,
                digest: None,
                wiki_compile_hash: None,
                wiki_assessed_hash: None,
                clean_content: None,
            }
        };

        let short_text = make_item("text", "短文本", None);
        let long_text = make_item("text", &"很长的文本内容".repeat(50), None);
        let noted_text = make_item("text", "有备注的内容", Some("这个很重要"));

        let prefs: Vec<(&str, f64)> = vec![];

        let s1 = compute_importance(&short_text, &prefs);
        let s2 = compute_importance(&long_text, &prefs);
        let s3 = compute_importance(&noted_text, &prefs);

        assert!(s2 > s1, "Longer text should score higher: {} vs {}", s2, s1);
        assert!(s3 > s1, "Noted text should score higher: {} vs {}", s3, s1);
    }
}
