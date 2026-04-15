//! OpenAI OAuth 2.0 module with PKCE flow for Codex API authentication.
//!
//! Implements:
//! - PKCE code verifier / challenge generation
//! - Local HTTP callback server (pure tokio async I/O, port 1455)
//! - Full login flow: open browser → wait for callback → exchange code
//! - Token refresh using refresh_token
//! - Token storage in memory (global state) + SQLite via repository
//! - JWT payload decoding to extract email + chatgpt_account_id

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

// Public OAuth client ID for a desktop/installed application.
// This is non-confidential and identifies the app during the OAuth browser redirect flow.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const SCOPES: &str = "openid profile email offline_access";
const CALLBACK_PORT: u16 = 1455;
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const DB_KEY: &str = "openai_oauth_token";

// ── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: i64,
    pub account_id: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub expires_at: Option<i64>,
}

// ── Global in-memory state ─────────────────────────────────────────────────

pub static OAUTH_STATE: Lazy<Arc<Mutex<Option<OAuthToken>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

// ── PKCE helpers ──────────────────────────────────────────────────────────

/// Generate a random PKCE code verifier (32 random bytes → base64url, no padding).
/// Uses a simple xorshift64 seeded from system time + PID since `rand` is not in
/// the main dependencies (only dev-dependencies).
fn generate_code_verifier() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let pid = std::process::id() as u64;
    // Mix in a per-call counter for extra uniqueness when called quickly in succession
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let cnt = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut state = seed ^ (pid.wrapping_shl(32) | pid.wrapping_shr(32)) ^ cnt;
    let mut bytes = [0u8; 32];
    for b in bytes.iter_mut() {
        // xorshift64
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
/// `/auth/callback?code=…`, return the `code` and `state` query parameters.
async fn wait_for_callback() -> Result<(String, String), String> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT))
        .await
        .map_err(|e| format!("Cannot bind to port {}: {}", CALLBACK_PORT, e))?;

    log::info!("OAuth callback server listening on port {}", CALLBACK_PORT);

    let (stream, _addr) =
        tokio::time::timeout(std::time::Duration::from_secs(180), listener.accept())
            .await
            .map_err(|_| "等待授权超时（180秒），请重试".to_string())?
            .map_err(|e| format!("Failed to accept connection: {}", e))?;

    let (reader_half, writer_half) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let mut writer = writer_half;

    // Read the request line (e.g. "GET /auth/callback?code=xxx&state=yyy HTTP/1.1")
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| format!("Failed to read request line: {}", e))?;

    log::debug!("OAuth callback request line: {}", request_line.trim());

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
    let body = b"<html><body><h2>Login successful! You can close this tab.</h2></body></html>";
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
    // Format: GET /auth/callback?code=XXX&state=YYY HTTP/1.1
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

// ── Token exchange & refresh ───────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    id_token: Option<String>,
}

/// Exchange an authorization code for tokens.
async fn exchange_code(code: &str, verifier: &str) -> Result<OAuthToken, String> {
    let client = reqwest::Client::new();
    let body = format!(
        "grant_type=authorization_code&client_id={}&code={}&redirect_uri={}&code_verifier={}",
        CLIENT_ID,
        url_encode(code),
        url_encode(REDIRECT_URI),
        url_encode(verifier),
    );

    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
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

    parse_token_response(token_resp)
}

/// Refresh an access token using the stored refresh_token.
pub async fn refresh_token(refresh: &str) -> Result<OAuthToken, String> {
    let client = reqwest::Client::new();
    let body = format!(
        "grant_type=refresh_token&client_id={}&refresh_token={}",
        CLIENT_ID,
        url_encode(refresh),
    );

    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
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

    parse_token_response(token_resp)
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

/// Convert a TokenResponse into an OAuthToken, decoding the id_token JWT for
/// user identity fields.
fn parse_token_response(resp: TokenResponse) -> Result<OAuthToken, String> {
    let expires_in = resp.expires_in.unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    // Decode id_token (JWT) to extract email and chatgpt_account_id
    let (email, account_id) = if let Some(id_token) = &resp.id_token {
        decode_jwt_payload(id_token).unwrap_or_default()
    } else {
        (String::new(), String::new())
    };

    let refresh = resp.refresh_token.unwrap_or_default();

    Ok(OAuthToken {
        access_token: resp.access_token,
        refresh_token: refresh,
        expires_at,
        account_id,
        email,
    })
}

// ── JWT payload decoding ───────────────────────────────────────────────────

/// Decode the payload section of a JWT and extract (email, chatgpt_account_id).
/// Returns `None` on any error (malformed JWT, missing fields, etc.).
fn decode_jwt_payload(jwt: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    // Add padding back if needed (base64url allows omitting '=')
    let payload_b64 = parts[1];
    let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    let email = json
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let account_id = json
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Some((email, account_id))
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Full OAuth login flow:
/// 1. Generate PKCE verifier + challenge.
/// 2. Open the browser to the authorization URL.
/// 3. Wait for the local callback with the auth code.
/// 4. Exchange the code for tokens.
pub async fn start_oauth_login() -> Result<OAuthToken, String> {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state_param = generate_code_verifier(); // reuse PKCE generator for random state

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        AUTH_URL,
        url_encode(CLIENT_ID),
        url_encode(REDIRECT_URI),
        url_encode(SCOPES),
        url_encode(&state_param),
        url_encode(&challenge),
    );

    log::info!("Opening browser for OAuth login: {}", auth_url);

    // Open browser — log warning on failure but don't abort (user can copy URL)
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
    let token = exchange_code(&code, &verifier).await?;
    log::info!(
        "OAuth login successful for email={}, account_id={}",
        token.email,
        token.account_id
    );

    Ok(token)
}

/// Persist token to SQLite and update the in-memory global state.
/// Accepts `Arc<Database>` to match the pattern used throughout the codebase.
pub async fn save_token(db: Arc<Database>, token: &OAuthToken) {
    // Update memory
    {
        let mut guard = OAUTH_STATE.lock().unwrap();
        *guard = Some(token.clone());
    }

    // Persist to DB
    let repo = Repository::new(db);
    match serde_json::to_string(token) {
        Ok(json) => {
            if let Err(e) = repo.update_setting(DB_KEY, &json) {
                log::error!("Failed to persist OAuth token to DB: {}", e);
            }
        }
        Err(e) => log::error!("Failed to serialize OAuth token: {}", e),
    }
}

/// Clear the stored token from memory and DB (logout).
/// Accepts `Arc<Database>` to match the pattern used throughout the codebase.
pub async fn clear_token(db: Arc<Database>) {
    {
        let mut guard = OAUTH_STATE.lock().unwrap();
        *guard = None;
    }

    let repo = Repository::new(db);
    if let Err(e) = repo.update_setting(DB_KEY, "") {
        log::error!("Failed to clear OAuth token from DB: {}", e);
    }
}

/// Load token from DB into the in-memory global state (call once at app startup).
pub fn load_token_from_db(db: Arc<Database>) {
    let repo = Repository::new(db);
    match repo.get_setting(DB_KEY) {
        Ok(Some(json)) if !json.is_empty() => match serde_json::from_str::<OAuthToken>(&json) {
            Ok(token) => {
                let mut guard = OAUTH_STATE.lock().unwrap();
                *guard = Some(token);
                log::info!("Loaded OAuth token from DB");
            }
            Err(e) => log::warn!("Failed to deserialize OAuth token from DB: {}", e),
        },
        Ok(_) => {}
        Err(e) => log::warn!("Failed to read OAuth token from DB: {}", e),
    }
}

/// Get a valid access token and account_id, loading from DB if memory is empty,
/// and auto-refreshing if the token expires within 5 minutes.
///
/// Returns `None` if not logged in or if refresh fails.
///
/// The `db` parameter is `Arc<Database>` to match the pattern used throughout
/// the codebase.  The public-facing commands will pass `state.db.clone()`.
pub async fn get_valid_token(db: Arc<Database>) -> Option<(String, String)> {
    // Try memory first
    let token_opt = {
        let guard = OAUTH_STATE.lock().unwrap();
        guard.clone()
    };

    // If not in memory, try to load from DB
    let token = match token_opt {
        Some(t) => t,
        None => {
            load_token_from_db(db.clone());
            let guard = OAUTH_STATE.lock().unwrap();
            guard.clone()?
        }
    };

    let now = chrono::Utc::now().timestamp();
    let five_minutes = 5 * 60;

    if token.expires_at - now > five_minutes {
        // Token is still fresh
        return Some((token.access_token, token.account_id));
    }

    // Need to refresh
    log::info!("OAuth token expiring soon, refreshing...");
    match refresh_token(&token.refresh_token).await {
        Ok(new_token) => {
            let result = (new_token.access_token.clone(), new_token.account_id.clone());
            save_token(db, &new_token).await;
            Some(result)
        }
        Err(e) => {
            log::error!("Token refresh failed: {}", e);
            None
        }
    }
}

/// Get the current OAuth status (for display in the settings UI).
pub fn get_oauth_status() -> OAuthStatus {
    let guard = OAUTH_STATE.lock().unwrap();
    match guard.as_ref() {
        Some(t) => OAuthStatus {
            logged_in: true,
            email: Some(t.email.clone()),
            expires_at: Some(t.expires_at),
        },
        None => OAuthStatus {
            logged_in: false,
            email: None,
            expires_at: None,
        },
    }
}
