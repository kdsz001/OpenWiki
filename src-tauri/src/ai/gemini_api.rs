use reqwest::Client;
use std::time::Duration;

const GEMINI_API_ENDPOINT: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse";

pub async fn call_gemini_api(
    access_token: &str,
    project_id: &str,
    model: &str,
    system_prompt: &str,
    user_message: &str,
    temperature: f32,
) -> Result<String, String> {
    let http_client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let request_id = format!("agent-{}", uuid::Uuid::new_v4());

    let sys = if system_prompt.is_empty() {
        "You are a helpful assistant."
    } else {
        system_prompt
    };

    let body = serde_json::json!({
        "project": project_id,
        "model": model,
        "request": {
            "contents": [{"role": "user", "parts": [{"text": user_message}]}],
            "generationConfig": {"maxOutputTokens": 16000, "temperature": temperature},
            "systemInstruction": {"role": "user", "parts": [{"text": sys}]}
        },
        "requestType": "agent",
        "userAgent": "antigravity",
        "requestId": request_id
    });

    let resp = http_client
        .post(GEMINI_API_ENDPOINT)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity/1.18.3 darwin/arm64")
        .header("Accept", "text/event-stream")
        .header(
            "X-Goog-Api-Client",
            "google-cloud-sdk vscode_cloudshelleditor/0.1",
        )
        .header(
            "Client-Metadata",
            r#"{"ideType":"ANTIGRAVITY","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#,
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini API request failed: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read Gemini response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Gemini API error ({}): {}", status, text));
    }

    let mut result = String::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                let candidates = event
                    .get("response")
                    .and_then(|r| r.get("candidates"))
                    .or_else(|| event.get("candidates"));
                if let Some(arr) = candidates.and_then(|c| c.as_array()) {
                    for candidate in arr {
                        if let Some(parts) = candidate
                            .get("content")
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.as_array())
                        {
                            for part in parts {
                                if part
                                    .get("thought")
                                    .and_then(|t| t.as_bool())
                                    .unwrap_or(false)
                                {
                                    continue;
                                }
                                if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                                    result.push_str(t);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if result.is_empty() {
        Err("Gemini API returned empty response".to_string())
    } else {
        log::info!(
            "Gemini API call successful, response length: {}",
            result.len()
        );
        Ok(result)
    }
}
