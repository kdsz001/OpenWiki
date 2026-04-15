//! Google Gemini OAuth 2.0 module with PKCE flow (Antigravity / Code Assist).
//!
//! Implements:
//! - PKCE code verifier / challenge generation
//! - Local HTTP callback server (pure tokio async I/O, port 51121)
//! - Full login flow: open browser → wait for callback → exchange code
//! - Token refresh using refresh_token + client_secret
//! - Fetch user email via userinfo endpoint
//! - Fetch projectId via loadCodeAssist endpoint
//! - Token storage in memory (global state) + SQLite via repository

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::storage::database::Database;
use crate::storage::repository::Repository;

// ── Constants ──────────────────────────────────────────────────────────────

// These are public OAuth client credentials for a desktop/installed application.
// Google considers these non-confidential for native apps (see Google OAuth docs).
// They identify the app to Google so users can authorize via browser redirect.
const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REDIRECT_URI: &str = "http://localhost:51121/oauth-callback";
const CALLBACK_PORT: u16 = 51121;
const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
const LOAD_CODE_ASSIST_ENDPOINT: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const DEFAULT_PROJECT_ID: &str = "rising-fact-p41fc";
const DB_KEY: &str = "gemini_oauth_token";

// ── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiOAuthToken {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: i64,
    pub project_id: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeminiOAuthStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub expires_at: Option<i64>,
}

// ── Global in-memory state ─────────────────────────────────────────────────

pub static GEMINI_OAUTH_STATE: Lazy<Arc<Mutex<Option<GeminiOAuthToken>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

// ── PKCE helpers ──────────────────────────────────────────────────────────

/// Generate a random PKCE code verifier (32 random bytes → base64url, no padding).
fn generate_code_verifier() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let pid = std::process::id() as u64;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let cnt = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut state = seed ^ (pid.wrapping_shl(32) | pid.wrapping_shr(32)) ^ cnt;
    let mut bytes = [0u8; 32];
    for b in bytes.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *b = (state & 0xff) as u8;
    }
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Compute the PKCE code challenge: SHA-256(verifier) → base64url, no padding.
fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

// ── Local callback server ─────────────────────────────────────────────────

/// Spin up a local HTTP server on CALLBACK_PORT, wait for one GET request to
/// `/oauth-callback?code=…`, return the `code` and `state` query parameters.
async fn wait_for_callback() -> Result<(String, String), String> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT))
        .await
        .map_err(|e| format!("Cannot bind to port {}: {}", CALLBACK_PORT, e))?;

    log::info!(
        "Gemini OAuth callback server listening on port {}",
        CALLBACK_PORT
    );

    let (stream, _addr) =
        tokio::time::timeout(std::time::Duration::from_secs(180), listener.accept())
            .await
            .map_err(|_| "等待授权超时（180秒），请重试".to_string())?
            .map_err(|e| format!("Failed to accept connection: {}", e))?;

    let (reader_half, writer_half) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let mut writer = writer_half;

    // Read the request line (e.g. "GET /oauth-callback?code=xxx&state=yyy HTTP/1.1")
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| format!("Failed to read request line: {}", e))?;

    log::debug!(
        "Gemini OAuth callback request line: {}",
        request_line.trim()
    );

    // Drain all remaining headers until blank line
    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .await
            .map_err(|e| format!("Failed to read header: {}", e))?;
        if header_line == "\r\n" || header_line.is_empty() {
            break;
        }
    }

    // Write HTTP response so the browser shows a friendly message
    let body =
        b"<html><body><h2>Google login successful! You can close this tab.</h2></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    writer
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("Failed to write response headers: {}", e))?;
    writer
        .write_all(body)
        .await
        .map_err(|e| format!("Failed to write response body: {}", e))?;
    writer
        .shutdown()
        .await
        .map_err(|e| format!("Failed to shutdown writer: {}", e))?;

    // Parse code and state from request line
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or("Malformed request line")?;

    let query = path
        .splitn(2, '?')
        .nth(1)
        .ok_or("No query string in callback URL")?;

    let mut code = String::new();
    let mut state = String::new();
    for param in query.split('&') {
        if let Some(v) = param.strip_prefix("code=") {
            code = url_decode(v);
        } else if let Some(v) = param.strip_prefix("state=") {
            state = url_decode(v);
        }
    }

    if code.is_empty() {
        return Err("No 'code' parameter in callback URL".into());
    }

    Ok((code, state))
}

/// Minimal URL percent-decode (handles %XX sequences and + as space).
fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.bytes().peekable();
    while let Some(b) = iter.next() {
        match b {
            b'+' => out.push(' '),
            b'%' => {
                let hi = iter.next().unwrap_or(b'0');
                let lo = iter.next().unwrap_or(b'0');
                let hex = [hi, lo];
                if let Ok(s) = std::str::from_utf8(&hex) {
                    if let Ok(n) = u8::from_str_radix(s, 16) {
                        out.push(n as char);
                        continue;
                    }
                }
                out.push('%');
                out.push(hi as char);
                out.push(lo as char);
            }
            _ => out.push(b as char),
        }
    }
    out
}

/// Minimal percent-encoder (encodes characters that must be encoded in form bodies).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ── Token exchange & refresh ───────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

/// Exchange an authorization code for tokens (includes client_secret for Google).
async fn exchange_code(
    code: &str,
    verifier: &str,
) -> Result<(String, Option<String>, i64), String> {
    let client = reqwest::Client::new();
    let body = format!(
        "client_id={}&client_secret={}&code={}&grant_type=authorization_code&redirect_uri={}&code_verifier={}",
        url_encode(CLIENT_ID),
        url_encode(CLIENT_SECRET),
        url_encode(code),
        url_encode(REDIRECT_URI),
        url_encode(verifier),
    );

    let resp = client
        .post(TOKEN_URL)
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header("User-Agent", "google-api-nodejs-client/9.15.1")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed ({}): {}", status, text));
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let expires_in = token_resp.expires_in.unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    Ok((
        token_resp.access_token,
        token_resp.refresh_token,
        expires_at,
    ))
}

/// Refresh an access token using the stored refresh_token (includes client_secret).
pub async fn refresh_gemini_token(refresh: &str) -> Result<GeminiOAuthToken, String> {
    let client = reqwest::Client::new();
    let body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}&client_secret={}",
        url_encode(refresh),
        url_encode(CLIENT_ID),
        url_encode(CLIENT_SECRET),
    );

    let resp = client
        .post(TOKEN_URL)
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header("User-Agent", "google-api-nodejs-client/9.15.1")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Token refresh failed ({}): {}", status, text));
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh token response: {}", e))?;

    let expires_in = token_resp.expires_in.unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    // For refresh, we keep the existing refresh_token if none returned
    let new_refresh = token_resp
        .refresh_token
        .unwrap_or_else(|| refresh.to_string());

    // Fetch user email and project_id with the new access token
    let email = fetch_user_email(&token_resp.access_token)
        .await
        .unwrap_or_default();
    let project_id = fetch_project_id(&token_resp.access_token)
        .await
        .unwrap_or_else(|| DEFAULT_PROJECT_ID.to_string());

    Ok(GeminiOAuthToken {
        access_token: token_resp.access_token,
        refresh_token: new_refresh,
        expires_at,
        project_id,
        email,
    })
}

// ── Post-auth helpers ──────────────────────────────────────────────────────

/// Fetch the authenticated user's email address from Google userinfo endpoint.
async fn fetch_user_email(access_token: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://www.googleapis.com/oauth2/v1/userinfo?alt=json")
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        log::warn!("Failed to fetch user email: {}", resp.status());
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Fetch the Google Cloud project ID via the loadCodeAssist endpoint.
/// Falls back to DEFAULT_PROJECT_ID if the call fails.
async fn fetch_project_id(access_token: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "metadata": {
            "ideType": "ANTIGRAVITY",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        }
    });

    let resp = client
        .post(LOAD_CODE_ASSIST_ENDPOINT)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "google-api-nodejs-client/9.15.1")
        .header(
            "Client-Metadata",
            r#"{"ideType":"ANTIGRAVITY","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#,
        )
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        log::warn!(
            "loadCodeAssist failed ({}), using default project_id",
            resp.status()
        );
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;

    // cloudaicompanionProject can be a string or an object { id: string }
    let project = json.get("cloudaicompanionProject")?;
    if let Some(s) = project.as_str() {
        return Some(s.to_string());
    }
    if let Some(id) = project.get("id").and_then(|v| v.as_str()) {
        return Some(id.to_string());
    }

    None
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Full Google OAuth login flow:
/// 1. Generate PKCE verifier + challenge.
/// 2. Open the browser to the authorization URL (access_type=offline, prompt=consent).
/// 3. Wait for the local callback with the auth code.
/// 4. Exchange the code for tokens.
/// 5. Fetch user email and project_id.
pub async fn start_gemini_oauth_login() -> Result<GeminiOAuthToken, String> {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state_param = generate_code_verifier(); // reuse PKCE generator for random state

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        AUTH_URL,
        url_encode(CLIENT_ID),
        url_encode(REDIRECT_URI),
        url_encode(SCOPES),
        url_encode(&state_param),
        url_encode(&challenge),
    );

    log::info!("Opening browser for Gemini OAuth login: {}", auth_url);

    if let Err(e) = open::that(&auth_url) {
        log::warn!(
            "Failed to open browser automatically: {}. Auth URL: {}",
            e,
            auth_url
        );
    }

    // Wait for callback
    let (code, returned_state) = wait_for_callback().await?;

    // Validate state to prevent CSRF
    if returned_state != state_param {
        return Err("OAuth state mismatch — possible CSRF attack".into());
    }

    // Exchange code for tokens
    let (access_token, refresh_token_opt, expires_at) = exchange_code(&code, &verifier).await?;
    let refresh_token = refresh_token_opt.unwrap_or_default();

    // Fetch email and project_id
    let email = fetch_user_email(&access_token).await.unwrap_or_default();
    let project_id = fetch_project_id(&access_token)
        .await
        .unwrap_or_else(|| DEFAULT_PROJECT_ID.to_string());

    log::info!(
        "Gemini OAuth login successful for email={}, project_id={}",
        email,
        project_id
    );

    Ok(GeminiOAuthToken {
        access_token,
        refresh_token,
        expires_at,
        project_id,
        email,
    })
}

/// Persist token to SQLite and update the in-memory global state.
pub async fn save_token(db: Arc<Database>, token: &GeminiOAuthToken) {
    {
        let mut guard = GEMINI_OAUTH_STATE.lock().unwrap();
        *guard = Some(token.clone());
    }

    let repo = Repository::new(db);
    match serde_json::to_string(token) {
        Ok(json) => {
            if let Err(e) = repo.update_setting(DB_KEY, &json) {
                log::error!("Failed to persist Gemini OAuth token to DB: {}", e);
            }
        }
        Err(e) => log::error!("Failed to serialize Gemini OAuth token: {}", e),
    }
}

/// Clear the stored token from memory and DB (logout).
pub async fn clear_token(db: Arc<Database>) {
    {
        let mut guard = GEMINI_OAUTH_STATE.lock().unwrap();
        *guard = None;
    }

    let repo = Repository::new(db);
    if let Err(e) = repo.update_setting(DB_KEY, "") {
        log::error!("Failed to clear Gemini OAuth token from DB: {}", e);
    }
}

/// Load token from DB into the in-memory global state (call once at app startup).
pub fn load_token_from_db(db: Arc<Database>) {
    let repo = Repository::new(db);
    match repo.get_setting(DB_KEY) {
        Ok(Some(json)) if !json.is_empty() => {
            match serde_json::from_str::<GeminiOAuthToken>(&json) {
                Ok(token) => {
                    let mut guard = GEMINI_OAUTH_STATE.lock().unwrap();
                    *guard = Some(token);
                    log::info!("Loaded Gemini OAuth token from DB");
                }
                Err(e) => log::warn!("Failed to deserialize Gemini OAuth token from DB: {}", e),
            }
        }
        Ok(_) => {}
        Err(e) => log::warn!("Failed to read Gemini OAuth token from DB: {}", e),
    }
}

/// Get a valid access token and project_id, loading from DB if memory is empty,
/// and auto-refreshing if the token expires within 5 minutes.
///
/// Returns `None` if not logged in or if refresh fails.
pub async fn get_valid_token(db: Arc<Database>) -> Option<(String, String)> {
    // Try memory first
    let token_opt = {
        let guard = GEMINI_OAUTH_STATE.lock().unwrap();
        guard.clone()
    };

    // If not in memory, try to load from DB
    let token = match token_opt {
        Some(t) => t,
        None => {
            load_token_from_db(db.clone());
            let guard = GEMINI_OAUTH_STATE.lock().unwrap();
            guard.clone()?
        }
    };

    let now = chrono::Utc::now().timestamp();
    let five_minutes = 5 * 60;

    if token.expires_at - now > five_minutes {
        // Token is still fresh
        return Some((token.access_token, token.project_id));
    }

    // Need to refresh
    log::info!("Gemini OAuth token expiring soon, refreshing...");
    match refresh_gemini_token(&token.refresh_token).await {
        Ok(new_token) => {
            let result = (new_token.access_token.clone(), new_token.project_id.clone());
            save_token(db, &new_token).await;
            Some(result)
        }
        Err(e) => {
            log::error!("Gemini token refresh failed: {}", e);
            None
        }
    }
}

/// Get the current Gemini OAuth status (for display in the settings UI).
pub fn get_gemini_oauth_status() -> GeminiOAuthStatus {
    let guard = GEMINI_OAUTH_STATE.lock().unwrap();
    match guard.as_ref() {
        Some(t) => GeminiOAuthStatus {
            logged_in: true,
            email: Some(t.email.clone()),
            expires_at: Some(t.expires_at),
        },
        None => GeminiOAuthStatus {
            logged_in: false,
            email: None,
            expires_at: None,
        },
    }
}
