use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Supported AI providers
#[derive(Debug, Clone)]
pub enum AiProvider {
    Anthropic,
    OpenAi,
    OpenRouter,
}

impl AiProvider {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => AiProvider::OpenAi,
            "openrouter" => AiProvider::OpenRouter,
            _ => AiProvider::Anthropic,
        }
    }
}

/// AI API client supporting Anthropic Claude and OpenAI
pub struct AiClient {
    pub api_key: String,
    pub provider: AiProvider,
    pub model: String,
    http_client: Client,
}

// --- Anthropic request/response types ---

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    error: AnthropicErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    message: String,
}

// --- OpenAI request/response types ---

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAiError {
    error: OpenAiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorDetail {
    message: String,
}

// --- Client response ---

/// Unified response from AI APIs
#[derive(Debug, Clone)]
pub struct AiResponse {
    pub text: String,
    pub tokens_used: Option<i32>,
}

impl AiClient {
    pub fn new(api_key: String, provider: String, model: String) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        AiClient {
            api_key,
            provider: AiProvider::from_str(&provider),
            model,
            http_client,
        }
    }

    /// Send a message to the configured AI provider and return the response.
    pub async fn send_message(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<AiResponse, String> {
        match self.provider {
            AiProvider::Anthropic => self.call_anthropic(system_prompt, user_message).await,
            AiProvider::OpenAi => self.call_openai(system_prompt, user_message).await,
            AiProvider::OpenRouter => self.call_openrouter(system_prompt, user_message).await,
        }
    }

    /// Call the Anthropic Claude Messages API
    async fn call_anthropic(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<AiResponse, String> {
        log::info!("Calling Anthropic API with model: {}", self.model);

        let request_body = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            system: system_prompt.to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
            }],
        };

        let response = self
            .http_client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Anthropic API request failed: {}", e))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read Anthropic response: {}", e))?;

        if !status.is_success() {
            let error_msg = if let Ok(err) = serde_json::from_str::<AnthropicError>(&body) {
                err.error.message
            } else {
                body.clone()
            };
            log::error!("Anthropic API error ({}): {}", status, error_msg);
            return Err(format!("Anthropic API error ({}): {}", status, error_msg));
        }

        let parsed: AnthropicResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse Anthropic response: {} body: {}", e, body))?;

        let text = parsed
            .content
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default();

        let tokens_used = parsed
            .usage
            .map(|u| (u.input_tokens + u.output_tokens) as i32);

        log::info!(
            "Anthropic API response successful, tokens: {:?}",
            tokens_used
        );

        Ok(AiResponse { text, tokens_used })
    }

    /// Call the OpenAI Chat Completions API
    async fn call_openai(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<AiResponse, String> {
        log::info!("Calling OpenAI API with model: {}", self.model);

        let request_body = OpenAiRequest {
            model: self.model.clone(),
            messages: vec![
                OpenAiMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                OpenAiMessage {
                    role: "user".to_string(),
                    content: user_message.to_string(),
                },
            ],
            max_tokens: 4096,
            temperature: 0.3,
        };

        let response = self
            .http_client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("OpenAI API request failed: {}", e))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read OpenAI response: {}", e))?;

        if !status.is_success() {
            let error_msg = if let Ok(err) = serde_json::from_str::<OpenAiError>(&body) {
                err.error.message
            } else {
                body.clone()
            };
            log::error!("OpenAI API error ({}): {}", status, error_msg);
            return Err(format!("OpenAI API error ({}): {}", status, error_msg));
        }

        let parsed: OpenAiResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse OpenAI response: {} body: {}", e, body))?;

        let text = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let tokens_used = parsed.usage.map(|u| u.total_tokens as i32);

        log::info!("OpenAI API response successful, tokens: {:?}", tokens_used);

        Ok(AiResponse { text, tokens_used })
    }

    /// Call the OpenRouter API (OpenAI-compatible format)
    async fn call_openrouter(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<AiResponse, String> {
        log::info!("Calling OpenRouter API with model: {}", self.model);

        let request_body = OpenAiRequest {
            model: self.model.clone(),
            messages: vec![
                OpenAiMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                OpenAiMessage {
                    role: "user".to_string(),
                    content: user_message.to_string(),
                },
            ],
            max_tokens: 4096,
            temperature: 0.3,
        };

        let response = self
            .http_client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://openwiki.app")
            .header("X-Title", "Xiaoyun")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("OpenRouter API request failed: {}", e))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read OpenRouter response: {}", e))?;

        if !status.is_success() {
            let error_msg = if let Ok(err) = serde_json::from_str::<OpenAiError>(&body) {
                err.error.message
            } else {
                body.clone()
            };
            log::error!("OpenRouter API error ({}): {}", status, error_msg);
            return Err(format!("OpenRouter API error ({}): {}", status, error_msg));
        }

        let parsed: OpenAiResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse OpenRouter response: {} body: {}", e, body))?;

        let text = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let tokens_used = parsed.usage.map(|u| u.total_tokens as i32);

        log::info!(
            "OpenRouter API response successful, tokens: {:?}",
            tokens_used
        );

        Ok(AiResponse { text, tokens_used })
    }
}
