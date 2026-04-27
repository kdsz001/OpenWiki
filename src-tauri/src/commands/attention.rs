use crate::ai::attention_analyzer::{self, AnalysisProvider};
use crate::commands::capture::AppState;
use crate::storage::models::AttentionInsight;
use crate::storage::repository::Repository;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarStatus {
    pub status: String,
    pub insight: Option<AttentionInsight>,
    pub has_new_content: bool,
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

    // OpenAI and Google can use OAuth instead of an API key;
    // Ollama and custom local providers don't need one at all.
    let oauth_provider = provider_str_check == "openai" || provider_str_check == "google";
    let is_local_or_custom = provider_str_check == "ollama" || provider_str_check == "custom" || provider_str_check == "lmstudio";
    if api_key.is_empty() && !oauth_provider && !is_local_or_custom {
        return Ok(RadarStatus {
            status: "no_api_key".to_string(),
            insight: None,
            has_new_content: false,
        });
    }

    // 2. Check if we have enough content (at least 5 items in the last 15 days)
    let content_check = repo
        .get_recent_content_for_analysis(15, 5)
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
            // Check if currently analyzing — but detect stale "analyzing".
            // Cloud providers are usually done within a minute; local models
            // (Ollama/custom) can genuinely take 5–15 minutes on consumer hardware,
            // so we use a longer threshold for them to avoid false-timeout flips.
            if insight.status == "analyzing" {
                let analyzed_time = chrono::DateTime::parse_from_rfc3339(&insight.analyzed_at)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let elapsed_min = (chrono::Utc::now() - analyzed_time).num_minutes();
                let stale_threshold_min: i64 = if is_local_or_custom { 15 } else { 5 };

                if elapsed_min > stale_threshold_min {
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
    use tauri::Emitter;

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

    // Allow OpenAI/Google/custom/ollama providers to proceed without an API key
    let is_local_or_custom = provider_str == "custom" || provider_str == "ollama" || provider_str == "lmstudio";
    if api_key.is_empty() && provider_str != "openai" && provider_str != "google" && !is_local_or_custom {
        return Err("Please configure an AI API Key in settings first".to_string());
    }

    let model = repo
        .get_setting("ai_model")
        .map_err(|e| format!("Failed to read AI model: {}", e))?
        .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

    let base_url = repo.get_setting("ai_custom_base_url").ok().flatten().unwrap_or_default();

    // 3. Get content for analysis (15 days, max 100)
    let items = repo
        .get_recent_content_for_analysis(15, 100)
        .map_err(|e| format!("Failed to get content: {}", e))?;

    if items.is_empty() {
        return Err("Not enough content for analysis".to_string());
    }

    let item_count = items.len();
    let provider = AnalysisProvider::from_str_with_base(&provider_str, &base_url);

    // 4. Create "analyzing" record
    let now = chrono::Utc::now();
    let window_end = now.to_rfc3339();
    let window_start = (now - chrono::TimeDelta::days(15)).to_rfc3339();

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

    // 5. Build prompt — all providers use v3 RadarReport format
    let stats = Repository::get_content_stats(&items);
    let (system_prompt, user_message) = attention_analyzer::build_prompt_v2(&items, &stats);
    let is_dashscope = matches!(provider, AnalysisProvider::DashScope);

    tauri::async_runtime::spawn(async move {
        let repo = Repository::new(db.clone());
        let _ = app.emit("attention-analysis-progress", "thinking");

        // Try Codex OAuth first if provider is openai
        if provider_str == "openai" {
            if let Some(result) = attention_analyzer::try_codex_call(
                db.clone(),
                &system_prompt,
                &user_message,
                0.7,
                true, // deep insight report
            )
            .await
            {
                match result {
                    Ok(raw_response) => {
                        log::info!("Codex OAuth insight analysis succeeded");
                        let _ = app.emit("attention-analysis-progress", "generating");
                        match attention_analyzer::validate_radar_report(&raw_response) {
                            Ok(report) => {
                                let json_str =
                                    serde_json::to_string(&report).unwrap_or_default();
                                if let Err(e) = repo.update_insight_status(
                                    insight_id,
                                    "complete",
                                    Some(&json_str),
                                    None,
                                ) {
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
                                    "Insight report generated (Codex OAuth), analyzed {} items",
                                    item_count
                                );
                                let _ = app.emit("attention-analysis-complete", "complete");
                            }
                            Err(e) => {
                                log::error!("Insight report validation failed: {}", e);
                                let _ = repo.update_insight_status(
                                    insight_id, "error", None, Some(&e),
                                );
                                let _ = app.emit("attention-analysis-complete", "error");
                            }
                        }
                        return;
                    }
                    Err(e) => {
                        log::warn!("Codex OAuth failed, falling back to API Key: {}", e);
                        // Fall through to API key path below
                    }
                }
            }
        }

        // Try Gemini OAuth if provider is google
        if provider_str == "google" {
            if let Some(result) = attention_analyzer::try_gemini_call(
                db.clone(),
                &system_prompt,
                &user_message,
                0.7,
                true, // deep insight report
            )
            .await
            {
                match result {
                    Ok(raw_response) => {
                        log::info!("Gemini OAuth insight analysis succeeded");
                        let _ = app.emit("attention-analysis-progress", "generating");
                        match attention_analyzer::validate_radar_report(&raw_response) {
                            Ok(report) => {
                                let json_str =
                                    serde_json::to_string(&report).unwrap_or_default();
                                if let Err(e) = repo.update_insight_status(
                                    insight_id,
                                    "complete",
                                    Some(&json_str),
                                    None,
                                ) {
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
                                    "Insight report generated (Gemini OAuth), analyzed {} items",
                                    item_count
                                );
                                let _ = app.emit("attention-analysis-complete", "complete");
                            }
                            Err(e) => {
                                log::error!("Insight report validation failed: {}", e);
                                let _ = repo.update_insight_status(
                                    insight_id, "error", None, Some(&e),
                                );
                                let _ = app.emit("attention-analysis-complete", "error");
                            }
                        }
                        return;
                    }
                    Err(e) => {
                        log::warn!("Gemini OAuth insight analysis failed, falling back to API Key: {}", e);
                        // Fall through to API key path below
                    }
                }
            }
        }

        // If we reach here and have no API key, report an error —
        // unless we're on a local provider (Ollama/custom) which doesn't need one.
        if api_key.is_empty() && !is_local_or_custom {
            log::error!("No available AI call method (no API Key, no OAuth Token)");
            let _ = repo.update_insight_status(
                insight_id,
                "error",
                None,
                Some("Please configure an API Key or log in via OAuth first"),
            );
            let _ = app.emit("attention-analysis-complete", "error");
            return;
        }

        // DashScope uses SSE streaming + thinking mode; others use standard API call
        let api_result = if is_dashscope {
            attention_analyzer::call_dashscope_streaming(
                &api_key,
                &model,
                &system_prompt,
                &user_message,
                8192,
            )
            .await
        } else {
            attention_analyzer::call_analysis_api(
                &provider,
                &api_key,
                &model,
                &system_prompt,
                &user_message,
                8192,
                true,
            )
            .await
        };

        match api_result {
            Ok(raw_response) => {
                let _ = app.emit("attention-analysis-progress", "generating");

                match attention_analyzer::validate_radar_report(&raw_response) {
                    Ok(report) => {
                        let json_str = serde_json::to_string(&report).unwrap_or_default();

                        if let Err(e) = repo.update_insight_status(
                            insight_id,
                            "complete",
                            Some(&json_str),
                            None,
                        ) {
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

                        log::info!("Insight report generated, analyzed {} items", item_count);
                        let _ = app.emit("attention-analysis-complete", "complete");
                    }
                    Err(e) => {
                        log::error!("Insight report validation failed: {}", e);
                        let _ = repo.update_insight_status(insight_id, "error", None, Some(&e));
                        let _ = app.emit("attention-analysis-complete", "error");
                    }
                }
            }
            Err(e) => {
                log::error!("AI API call failed: {}", e);
                let _ = repo.update_insight_status(insight_id, "error", None, Some(&e));
                let _ = app.emit("attention-analysis-complete", "error");
            }
        }
    });

    Ok(())
}
