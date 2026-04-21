use crate::ai::attention_analyzer::{self, AnalysisProvider};
use crate::capture::content::compute_hash;
use crate::commands::capture::AppState;
use crate::storage::database::Database;
use crate::storage::models::{AttentionInsight, ContentForAnalysis};
use crate::storage::repository::Repository;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

const RADAR_BATCH_SIZE: usize = 100;
const RADAR_AUTO_RECENT_ITEMS: usize = 80;
const RADAR_BATCH_CACHE_SCHEMA: &str = "radar-batch-v1";

fn read_radar_analysis_days(repo: &Repository) -> i64 {
    repo.get_setting("radar_analysis_days")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .filter(|v: &i64| *v >= 0)
        .unwrap_or(100)
}

fn read_radar_analysis_limit(repo: &Repository) -> usize {
    repo.get_setting("radar_analysis_limit")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarStatus {
    pub status: String,
    pub insight: Option<AttentionInsight>,
    pub has_new_content: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarProgressPayload {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub selected_items: Option<usize>,
    pub total_items: Option<usize>,
}

fn compute_analysis_item_signature(item: &ContentForAnalysis) -> String {
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        item.id,
        item.captured_at,
        item.source_app,
        item.content_type,
        item.source_url.as_deref().unwrap_or(""),
        item.summary.as_deref().unwrap_or(""),
        item.tags.as_deref().unwrap_or(""),
        item.user_note.as_deref().unwrap_or(""),
        item.raw_text.as_deref().unwrap_or(""),
    );
    compute_hash(payload.as_bytes())
}

fn build_attention_batch_key(
    items: &[ContentForAnalysis],
    provider: &str,
    model: &str,
    locale: &str,
) -> String {
    let mut payload = format!(
        "{}|{}|{}|{}|{}|",
        RADAR_BATCH_CACHE_SCHEMA,
        provider,
        model,
        locale,
        items.len()
    );
    for item in items {
        payload.push_str(&compute_analysis_item_signature(item));
        payload.push('|');
    }
    compute_hash(payload.as_bytes())
}

fn build_attention_batches(items: &[ContentForAnalysis]) -> Vec<Vec<ContentForAnalysis>> {
    items
        .chunks(RADAR_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

fn auto_radar_item_budget(total_items: usize) -> usize {
    match total_items {
        0..=120 => total_items,
        121..=500 => 120,
        501..=2_000 => 160,
        2_001..=5_000 => 200,
        _ => 240,
    }
}

fn select_radar_items(
    items: &[ContentForAnalysis],
    configured_limit: usize,
) -> Vec<ContentForAnalysis> {
    if configured_limit > 0 {
        return items.iter().take(configured_limit).cloned().collect();
    }

    let budget = auto_radar_item_budget(items.len());
    if items.len() <= budget {
        return items.to_vec();
    }

    let recent_target = budget.min(RADAR_AUTO_RECENT_ITEMS);
    let mut selected = Vec::with_capacity(budget);
    let mut seen_ids = HashSet::with_capacity(budget);

    for item in items.iter().take(recent_target) {
        if seen_ids.insert(item.id.clone()) {
            selected.push(item.clone());
        }
    }

    let remaining_budget = budget.saturating_sub(selected.len());
    let tail = &items[recent_target..];

    if remaining_budget > 0 && !tail.is_empty() {
        for idx in 0..remaining_budget {
            let pos = idx * tail.len() / remaining_budget;
            if let Some(item) = tail.get(pos).or_else(|| tail.last()) {
                if seen_ids.insert(item.id.clone()) {
                    selected.push(item.clone());
                }
            }
        }

        if selected.len() < budget {
            for item in tail {
                if seen_ids.insert(item.id.clone()) {
                    selected.push(item.clone());
                    if selected.len() >= budget {
                        break;
                    }
                }
            }
        }
    }

    selected
}

fn extract_stat_u32(stats: &serde_json::Value, key: &str) -> u32 {
    stats.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u32
}

fn format_short_date_range(items: &[ContentForAnalysis]) -> String {
    let min_day = items
        .iter()
        .filter_map(|item| item.captured_at.get(..10))
        .min()
        .unwrap_or("");
    let max_day = items
        .iter()
        .filter_map(|item| item.captured_at.get(..10))
        .max()
        .unwrap_or("");
    if min_day.is_empty() || max_day.is_empty() {
        return String::new();
    }
    if min_day == max_day {
        return min_day.to_string();
    }
    format!("{}~{}", &min_day[5..], &max_day[5..])
}

fn build_exact_heatmap(items: &[ContentForAnalysis]) -> Vec<attention_analyzer::HeatmapDay> {
    use std::collections::BTreeMap;

    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for item in items {
        if let Some(day) = item.captured_at.get(..10) {
            *counts.entry(day.to_string()).or_default() += 1;
        }
    }
    let peak = counts.values().copied().max().unwrap_or(0);
    counts
        .into_iter()
        .map(|(day, count)| attention_analyzer::HeatmapDay {
            date: if day.len() >= 10 {
                day[5..10].to_string()
            } else {
                day
            },
            count,
            is_peak: peak > 0 && count == peak,
        })
        .collect()
}

fn hydrate_radar_report(
    mut report: attention_analyzer::RadarReport,
    stats: &serde_json::Value,
    items: &[ContentForAnalysis],
) -> attention_analyzer::RadarReport {
    report.meta.date_range = stats
        .get("date_range")
        .and_then(|v| v.as_str())
        .unwrap_or(&report.meta.date_range)
        .to_string();
    report.meta.total_items = extract_stat_u32(stats, "total_items");
    report.meta.active_days = extract_stat_u32(stats, "active_days");
    report.meta.annotated_items = extract_stat_u32(stats, "annotated_items");
    report.meta.annotation_rate = stats
        .get("annotation_rate")
        .and_then(|v| v.as_str())
        .unwrap_or(&report.meta.annotation_rate)
        .to_string();
    report.meta.source_count = extract_stat_u32(stats, "source_count");
    report.footer.date_range = format_short_date_range(items);
    report.footer.total = extract_stat_u32(stats, "total_items");
    report.footer.active_days = extract_stat_u32(stats, "active_days");
    report.footer.total_days = extract_stat_u32(stats, "total_days");
    report.heatmap = build_exact_heatmap(items);
    report
}

async fn call_attention_model(
    db: Arc<Database>,
    provider_str: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_message: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let provider = AnalysisProvider::from_str(provider_str);

    if provider_str == "openai" {
        if let Some(result) =
            attention_analyzer::try_codex_call(db.clone(), system_prompt, user_message, 0.7, true)
                .await
        {
            return result.map_err(|e| format!("OpenAI OAuth 分析失败：{}", e));
        } else if api_key.is_empty() {
            return Err("OpenAI 登录已失效，请重新登录或填写 API Key".to_string());
        }
    }

    if provider_str == "google" {
        if let Some(result) =
            attention_analyzer::try_gemini_call(db.clone(), system_prompt, user_message, 0.7, true)
                .await
        {
            return result.map_err(|e| format!("Google OAuth 分析失败：{}", e));
        } else if api_key.is_empty() {
            return Err("Google 登录已失效，请重新登录或填写 API Key".to_string());
        }
    }

    if api_key.is_empty() {
        return Err("Please configure an API Key or log in via OAuth first".to_string());
    }

    if matches!(provider, AnalysisProvider::DashScope) {
        attention_analyzer::call_dashscope_streaming(
            api_key,
            model,
            system_prompt,
            user_message,
            max_tokens,
        )
        .await
    } else {
        attention_analyzer::call_analysis_api(
            &provider,
            api_key,
            model,
            system_prompt,
            user_message,
            max_tokens,
        )
        .await
    }
}

async fn parse_radar_report_with_repair(
    db: Arc<Database>,
    provider_str: &str,
    api_key: &str,
    model: &str,
    raw: &str,
) -> Result<attention_analyzer::RadarReport, String> {
    match attention_analyzer::validate_radar_report(raw) {
        Ok(report) => Ok(report),
        Err(initial_err) => {
            log::warn!("RadarReport parse failed, attempting repair: {}", initial_err);
            let (repair_system, repair_user) = attention_analyzer::build_radar_repair_prompt(raw);
            let repaired_raw = call_attention_model(
                db,
                provider_str,
                api_key,
                model,
                &repair_system,
                &repair_user,
                8192,
            )
            .await
            .map_err(|repair_err| {
                format!(
                    "{}; JSON repair request failed: {}",
                    initial_err, repair_err
                )
            })?;

            attention_analyzer::validate_radar_report(&repaired_raw).map_err(|repair_parse_err| {
                format!(
                    "{}; repaired JSON still invalid: {}",
                    initial_err, repair_parse_err
                )
            })
        }
    }
}

async fn build_batched_radar_report(
    app: &AppHandle,
    db: Arc<Database>,
    provider_str: &str,
    api_key: &str,
    model: &str,
    locale: &str,
    items: &[ContentForAnalysis],
) -> Result<attention_analyzer::RadarReport, String> {
    let repo = Repository::new(db.clone());
    let overall_stats = Repository::get_content_stats(items);
    let batches = build_attention_batches(items);
    let total_batches = batches.len();
    let mut batch_summaries = Vec::with_capacity(total_batches);
    let mut last_batch_report: Option<attention_analyzer::RadarReport> = None;

    for (index, batch) in batches.iter().enumerate() {
        let _ = app.emit(
            "attention-analysis-progress",
            RadarProgressPayload {
                stage: "batching".to_string(),
                current: index + 1,
                total: total_batches,
                selected_items: Some(items.len()),
                total_items: Some(items.len()),
            },
        );

        let batch_key = build_attention_batch_key(batch, provider_str, model, locale);
        let batch_stats = Repository::get_content_stats(batch);

        let batch_report = if let Some(cached_json) = repo
            .get_attention_batch_report(&batch_key)
            .map_err(|e| format!("Failed to read attention batch cache: {}", e))?
        {
            match attention_analyzer::validate_radar_report(&cached_json) {
                Ok(report) => report,
                Err(e) => {
                    log::warn!("Ignoring stale attention batch cache {}: {}", batch_key, e);
                    let (system_prompt, user_message) =
                        attention_analyzer::build_batch_prompt(batch, &batch_stats);
                    let raw = call_attention_model(
                        db.clone(),
                        provider_str,
                        api_key,
                        model,
                        &system_prompt,
                        &user_message,
                        8192,
                    )
                    .await?;
                    let report =
                        parse_radar_report_with_repair(db.clone(), provider_str, api_key, model, &raw)
                            .await?;
                    let report = hydrate_radar_report(report, &batch_stats, batch);
                    let report_json = serde_json::to_string(&report).unwrap_or_default();
                    let content_ids_json = serde_json::to_string(
                        &batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| "[]".to_string());
                    repo.save_attention_batch_report(
                        &batch_key,
                        &content_ids_json,
                        &report_json,
                        batch.len(),
                        model,
                        locale,
                    )
                    .map_err(|err| format!("Failed to save attention batch cache: {}", err))?;
                    report
                }
            }
        } else {
            let (system_prompt, user_message) =
                attention_analyzer::build_batch_prompt(batch, &batch_stats);
            let raw = call_attention_model(
                db.clone(),
                provider_str,
                api_key,
                model,
                &system_prompt,
                &user_message,
                8192,
            )
            .await?;
            let report =
                parse_radar_report_with_repair(db.clone(), provider_str, api_key, model, &raw)
                    .await?;
            let report = hydrate_radar_report(report, &batch_stats, batch);
            let report_json = serde_json::to_string(&report).unwrap_or_default();
            let content_ids_json = serde_json::to_string(
                &batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
            )
            .unwrap_or_else(|_| "[]".to_string());
            repo.save_attention_batch_report(
                &batch_key,
                &content_ids_json,
                &report_json,
                batch.len(),
                model,
                locale,
            )
            .map_err(|err| format!("Failed to save attention batch cache: {}", err))?;
            report
        };

        last_batch_report = Some(batch_report.clone());
        batch_summaries.push(attention_analyzer::compact_radar_report(&batch_report));
    }

    if total_batches == 1 {
        return Ok(hydrate_radar_report(
            last_batch_report.ok_or_else(|| "Missing batch report".to_string())?,
            &overall_stats,
            items,
        ));
    }

    let _ = app.emit(
        "attention-analysis-progress",
        RadarProgressPayload {
            stage: "aggregating".to_string(),
            current: total_batches,
            total: total_batches,
            selected_items: Some(items.len()),
            total_items: Some(items.len()),
        },
    );
    let (aggregate_system, aggregate_user) =
        attention_analyzer::build_aggregate_prompt(&batch_summaries, &overall_stats);
    let aggregate_raw = call_attention_model(
        db.clone(),
        provider_str,
        api_key,
        model,
        &aggregate_system,
        &aggregate_user,
        8192,
    )
    .await?;
    let aggregate_report =
        parse_radar_report_with_repair(db, provider_str, api_key, model, &aggregate_raw)
            .await?;
    Ok(hydrate_radar_report(
        aggregate_report,
        &overall_stats,
        items,
    ))
}

/// Get the current attention radar status and insight.
#[tauri::command]
pub fn get_attention_insights(state: State<'_, AppState>) -> Result<RadarStatus, String> {
    let repo = Repository::new(state.db.clone());

    // 1. Check if API key is configured (per-provider, with legacy fallback)
    let provider_str_check = repo
        .get_setting("ai_provider")
        .ok()
        .flatten()
        .unwrap_or_else(|| "anthropic".to_string());
    let api_key = repo
        .get_setting(&format!("ai_api_key_{}", provider_str_check))
        .ok()
        .flatten()
        .or_else(|| repo.get_setting("ai_api_key").ok().flatten())
        .unwrap_or_default();

    // OpenAI and Google can use OAuth instead of an API key
    let oauth_provider = provider_str_check == "openai" || provider_str_check == "google";
    if api_key.is_empty() && !oauth_provider {
        return Ok(RadarStatus {
            status: "no_api_key".to_string(),
            insight: None,
            has_new_content: false,
        });
    }

    let analysis_days = read_radar_analysis_days(&repo);

    // 2. Check if we have enough content (at least 5 items in the analysis window)
    let content_check = repo
        .get_recent_content_for_analysis(analysis_days, 5)
        .map_err(|e| format!("Failed to check content: {}", e))?;

    if content_check.len() < 5 {
        return Ok(RadarStatus {
            status: "not_enough_content".to_string(),
            insight: None,
            has_new_content: false,
        });
    }

    // 3. Get current insight
    let insight = repo
        .get_current_insight()
        .map_err(|e| format!("Failed to get insight: {}", e))?;

    match insight {
        None => Ok(RadarStatus {
            status: "empty".to_string(),
            insight: None,
            has_new_content: true,
        }),
        Some(insight) => {
            // Check if currently analyzing — but detect stale "analyzing" (>5 min = stuck)
            if insight.status == "analyzing" {
                let analyzed_time = chrono::DateTime::parse_from_rfc3339(&insight.analyzed_at)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let elapsed_min = (chrono::Utc::now() - analyzed_time).num_minutes();

                if elapsed_min > 5 {
                    // Stuck — reset to error so user can retry
                    let _ = repo.update_insight_status(
                        insight.id,
                        "error",
                        None,
                        Some("Analysis timed out, please retry"),
                    );
                    return Ok(RadarStatus {
                        status: "error".to_string(),
                        insight: Some(insight),
                        has_new_content: true,
                    });
                }

                return Ok(RadarStatus {
                    status: "analyzing".to_string(),
                    insight: Some(insight),
                    has_new_content: false,
                });
            }

            // Check if there's an error
            if insight.status == "error" {
                return Ok(RadarStatus {
                    status: "error".to_string(),
                    insight: Some(insight),
                    has_new_content: true,
                });
            }

            // Check if new content has arrived since the last analysis
            let has_new = repo
                .has_new_content_since(&insight.analyzed_at)
                .map_err(|e| format!("Failed to check for new content: {}", e))?;

            // Check if enough time has passed since last analysis (default: 3 days)
            let interval_days: i64 = repo
                .get_setting("radar_interval_days")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);

            let analyzed_time = chrono::DateTime::parse_from_rfc3339(&insight.analyzed_at)
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            let elapsed_days = (chrono::Utc::now() - analyzed_time).num_days();
            let interval_expired = elapsed_days >= interval_days;

            let status = if has_new && interval_expired {
                "stale"
            } else {
                "fresh"
            };

            Ok(RadarStatus {
                status: status.to_string(),
                insight: Some(insight),
                has_new_content: has_new,
            })
        }
    }
}

/// Trigger a new attention analysis in the background.
/// Uses v3 RadarReport for DashScope (SSE streaming + thinking),
/// falls back to v2 BriefingAnalysis for other providers.
#[tauri::command]
pub async fn trigger_attention_analysis(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    let repo = Repository::new(db.clone());

    // 1. Check if already analyzing
    let current = repo
        .get_current_insight()
        .map_err(|e| format!("Failed to check status: {}", e))?;

    if let Some(ref insight) = current {
        if insight.status == "analyzing" {
            return Ok(());
        }
    }

    // 2. Read AI settings
    let provider_str = repo
        .get_setting("ai_provider")
        .map_err(|e| format!("Failed to read AI provider: {}", e))?
        .unwrap_or_else(|| "anthropic".to_string());

    let api_key = repo
        .get_setting(&format!("ai_api_key_{}", provider_str))
        .ok()
        .flatten()
        .or_else(|| repo.get_setting("ai_api_key").ok().flatten())
        .unwrap_or_default();

    // Allow OpenAI/Google providers to proceed without an API key if OAuth is available
    if api_key.is_empty() && provider_str != "openai" && provider_str != "google" {
        return Err("Please configure an AI API Key in settings first".to_string());
    }

    let model = repo
        .get_setting("ai_model")
        .map_err(|e| format!("Failed to read AI model: {}", e))?
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());

    let analysis_days = read_radar_analysis_days(&repo);
    let analysis_limit = read_radar_analysis_limit(&repo);

    // 3. Get content for analysis (configurable days and sample limit; 0 = no limit)
    let fetched_items = repo
        .get_recent_content_for_analysis(analysis_days, analysis_limit)
        .map_err(|e| format!("Failed to get content: {}", e))?;

    if fetched_items.is_empty() {
        return Err("Not enough content for analysis".to_string());
    }

    let total_available = fetched_items.len();
    let items = select_radar_items(&fetched_items, analysis_limit);
    let selected_count = items.len();
    let total_batches = items.len().div_ceil(RADAR_BATCH_SIZE);

    let item_count = items.len();
    let locale = crate::locale::resolve_locale(&db);

    // 4. Create "analyzing" record
    let now = chrono::Utc::now();
    let window_end = now.to_rfc3339();
    let window_start = if analysis_days > 0 {
        (now - chrono::TimeDelta::days(analysis_days)).to_rfc3339()
    } else {
        items
            .iter()
            .filter_map(|item| chrono::DateTime::parse_from_rfc3339(&item.captured_at).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .min()
            .unwrap_or(now)
            .to_rfc3339()
    };

    let insight_id = repo
        .save_attention_insight(
            None,
            "analyzing",
            None,
            &window_start,
            &window_end,
            item_count as i32,
            &model,
        )
        .map_err(|e| format!("Failed to create analysis record: {}", e))?;

    let _ = app.emit(
        "attention-analysis-progress",
        RadarProgressPayload {
            stage: "planning".to_string(),
            current: 0,
            total: total_batches,
            selected_items: Some(selected_count),
            total_items: Some(total_available),
        },
    );

    tauri::async_runtime::spawn(async move {
        let repo = Repository::new(db.clone());
        let _ = app.emit(
            "attention-analysis-progress",
            RadarProgressPayload {
                stage: "thinking".to_string(),
                current: 0,
                total: total_batches,
                selected_items: Some(selected_count),
                total_items: Some(total_available),
            },
        );
        match build_batched_radar_report(
            &app,
            db.clone(),
            &provider_str,
            &api_key,
            &model,
            &locale,
            &items,
        )
        .await
        {
            Ok(report) => {
                let _ = app.emit(
                    "attention-analysis-progress",
                    RadarProgressPayload {
                        stage: "generating".to_string(),
                        current: total_batches,
                        total: total_batches,
                        selected_items: Some(selected_count),
                        total_items: Some(total_available),
                    },
                );
                let json_str = serde_json::to_string(&report).unwrap_or_default();
                if let Err(e) =
                    repo.update_insight_status(insight_id, "complete", Some(&json_str), None)
                {
                    log::error!("Failed to save insight report: {}", e);
                    let _ = repo.update_insight_status(
                        insight_id,
                        "error",
                        None,
                        Some(&format!("Failed to save: {}", e)),
                    );
                    let _ = app.emit("attention-analysis-complete", "error");
                    return;
                }
                log::info!(
                    "Insight report generated via batched pipeline, analyzed {} items",
                    item_count
                );
                let _ = app.emit("attention-analysis-complete", "complete");
            }
            Err(e) => {
                log::error!("Attention analysis failed: {}", e);
                let _ = repo.update_insight_status(insight_id, "error", None, Some(&e));
                let _ = app.emit("attention-analysis-complete", "error");
            }
        }
    });

    Ok(())
}
