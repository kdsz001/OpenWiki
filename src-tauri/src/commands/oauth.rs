use crate::ai::gemini_oauth;
use crate::ai::oauth;
use crate::commands::capture::AppState;
use tauri::State;

#[tauri::command]
pub async fn start_openai_oauth(state: State<'_, AppState>) -> Result<oauth::OAuthStatus, String> {
    let token = oauth::start_oauth_login().await?;
    oauth::save_token(state.db.clone(), &token).await;
    Ok(oauth::OAuthStatus {
        logged_in: true,
        email: Some(token.email),
        expires_at: Some(token.expires_at),
    })
}

#[tauri::command]
pub async fn get_openai_oauth_status(
    state: State<'_, AppState>,
) -> Result<oauth::OAuthStatus, String> {
    let valid = oauth::get_valid_token(state.db.clone()).await;
    match valid {
        Some(_) => {
            let guard = oauth::OAUTH_STATE.lock().unwrap();
            if let Some(token) = guard.as_ref() {
                Ok(oauth::OAuthStatus {
                    logged_in: true,
                    email: Some(token.email.clone()),
                    expires_at: Some(token.expires_at),
                })
            } else {
                Ok(oauth::OAuthStatus {
                    logged_in: false,
                    email: None,
                    expires_at: None,
                })
            }
        }
        None => Ok(oauth::OAuthStatus {
            logged_in: false,
            email: None,
            expires_at: None,
        }),
    }
}

#[tauri::command]
pub async fn logout_openai_oauth(state: State<'_, AppState>) -> Result<(), String> {
    oauth::clear_token(state.db.clone()).await;
    log::info!("OAuth: user logged out");
    Ok(())
}

#[tauri::command]
pub async fn start_gemini_oauth(
    state: State<'_, AppState>,
) -> Result<gemini_oauth::GeminiOAuthStatus, String> {
    let token = gemini_oauth::start_gemini_oauth_login().await?;
    gemini_oauth::save_token(state.db.clone(), &token).await;
    Ok(gemini_oauth::GeminiOAuthStatus {
        logged_in: true,
        email: Some(token.email),
        expires_at: Some(token.expires_at),
    })
}

#[tauri::command]
pub async fn get_gemini_oauth_status(
    state: State<'_, AppState>,
) -> Result<gemini_oauth::GeminiOAuthStatus, String> {
    let valid = gemini_oauth::get_valid_token(state.db.clone()).await;
    match valid {
        Some(_) => {
            let guard = gemini_oauth::GEMINI_OAUTH_STATE.lock().unwrap();
            if let Some(token) = guard.as_ref() {
                Ok(gemini_oauth::GeminiOAuthStatus {
                    logged_in: true,
                    email: Some(token.email.clone()),
                    expires_at: Some(token.expires_at),
                })
            } else {
                Ok(gemini_oauth::GeminiOAuthStatus {
                    logged_in: false,
                    email: None,
                    expires_at: None,
                })
            }
        }
        None => Ok(gemini_oauth::GeminiOAuthStatus {
            logged_in: false,
            email: None,
            expires_at: None,
        }),
    }
}

#[tauri::command]
pub async fn logout_gemini_oauth(state: State<'_, AppState>) -> Result<(), String> {
    gemini_oauth::clear_token(state.db.clone()).await;
    log::info!("Gemini OAuth: user logged out");
    Ok(())
}
