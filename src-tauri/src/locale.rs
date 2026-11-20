//! Unified locale resolver for the app.
//! Reads the user's language_mode from the database and resolves it to a concrete locale.
//! Used by AI prompts, background tasks, and any backend code that needs the current locale.

use crate::storage::database::Database;
use crate::storage::repository::Repository;
use std::sync::Arc;

/// Supported locales.
pub const LOCALE_ZH_CN: &str = "zh-CN";
pub const LOCALE_EN_US: &str = "en-US";

/// Resolve the current app locale from persisted settings.
/// Falls back to zh-CN if no setting is found.
pub fn resolve_locale(db: &Arc<Database>) -> String {
    let repo = Repository::new(db.clone());
    match repo.get_setting("language_mode") {
        Ok(Some(mode)) => resolve_from_mode(&mode),
        _ => LOCALE_ZH_CN.to_string(),
    }
}

/// Resolve a language mode string to a concrete locale.
fn resolve_from_mode(mode: &str) -> String {
    match mode {
        "en-US" => LOCALE_EN_US.to_string(),
        "zh-CN" => LOCALE_ZH_CN.to_string(),
        "system" => system_locale(),
        _ => LOCALE_ZH_CN.to_string(),
    }
}

/// Detect the user's true system language preference.
///
/// On macOS we MUST prefer the native `defaults` settings over the `LANG`
/// environment variable: Terminal and many GUI launch contexts export
/// `LANG=en_US.UTF-8` even when the system UI language is set to Chinese,
/// so trusting `LANG` causes AI summaries (and other locale-sensitive
/// output) to be silently generated in English on Chinese machines.
///
/// Priority order:
///   1. macOS `AppleLocale`         (simplest, e.g. "zh_CN" / "en_US")
///   2. macOS `AppleLanguages`      (user's ordered language list)
///   3. Windows system locale
///   4. `LANG` env var              (only reliable on Linux)
///   5. default → zh-CN
pub(crate) fn system_locale() -> String {
    #[cfg(target_os = "macos")]
    {
        // 1. AppleLocale — single-line output like "zh_CN\n"
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
        {
            let locale = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Some(locale) = normalize_system_locale(&locale) {
                return locale;
            }
        }

        // 2. AppleLanguages — multi-line plist array:
        //        (
        //            "zh-Hans-CN",
        //            "en-US"
        //        )
        //    We want the first quoted string — that's the user's top preference.
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
        {
            let raw = String::from_utf8_lossy(&output.stdout);
            if let Some(q1) = raw.find('"') {
                if let Some(q2_rel) = raw[q1 + 1..].find('"') {
                    let first_lang = &raw[q1 + 1..q1 + 1 + q2_rel];
                    if let Some(locale) = normalize_system_locale(first_lang) {
                        return locale;
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    if let Some(locale) =
        sys_locale::get_locale().and_then(|locale| normalize_system_locale(&locale))
    {
        return locale;
    }

    // 4. LANG env var — last resort. Reliable on Linux, but on macOS it
    //    often lies (see doc comment above).
    if let Ok(lang) = std::env::var("LANG") {
        if let Some(locale) = normalize_system_locale(&lang) {
            return locale;
        }
    }

    // 5. Default.
    LOCALE_ZH_CN.to_string()
}

fn normalize_system_locale(locale: &str) -> Option<String> {
    let locale = locale.trim().to_ascii_lowercase();
    if locale.starts_with("en") {
        Some(LOCALE_EN_US.to_string())
    } else if locale.starts_with("zh") {
        Some(LOCALE_ZH_CN.to_string())
    } else {
        None
    }
}

/// Check if the current locale is English.
pub fn is_english(locale: &str) -> bool {
    locale.starts_with("en")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_system_locales() {
        assert_eq!(normalize_system_locale("en-US"), Some("en-US".to_string()));
        assert_eq!(
            normalize_system_locale("en_US.UTF-8"),
            Some("en-US".to_string())
        );
        assert_eq!(normalize_system_locale("zh-CN"), Some("zh-CN".to_string()));
        assert_eq!(
            normalize_system_locale("zh-Hans"),
            Some("zh-CN".to_string())
        );
        assert_eq!(normalize_system_locale("fr-FR"), None);
    }
}
