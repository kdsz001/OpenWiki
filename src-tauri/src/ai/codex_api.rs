use reqwest::Client;
use std::time::Duration;

const CODEX_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

pub async fn call_codex_api(
    access_token: &str,
    account_id: &str,
    model: &str,
    instructions: &str,
    user_message: &str,
    temperature: f32,
) -> Result<String, String> {
    let http_client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let instructions_text = if instructions.is_empty() {
        "You are a helpful assistant."
    } else {
        instructions
    };

    let body = serde_json::json!({
        "model": model,
        "instructions": instructions_text,
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": user_message
                    }
                ]
            }
        ],
        "stream": true,
        "store": false
    });

    let resp = http_client
        .post(CODEX_API_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("ChatGPT-Account-Id", account_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Codex API request failed: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read Codex response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Codex API error ({}): {}", status, text));
    }

    // Parse SSE stream — accumulate response.output_text.delta events
    let mut result = String::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                break;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                if event["type"].as_str() == Some("response.output_text.delta") {
                    if let Some(delta) = event["delta"].as_str() {
                        result.push_str(delta);
                    }
                }
            }
        }
    }

    if result.is_empty() {
        Err("Codex API returned empty response".to_string())
    } else {
        log::info!(
            "Codex API call successful, response length: {}",
            result.len()
        );
        Ok(result)
    }
}
