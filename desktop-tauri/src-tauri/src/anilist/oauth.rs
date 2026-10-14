//! AniList OAuth — port of `anilist-oauth.ts` + `anilist-user.ts`.
//!
//! Sign-in opens the system browser; the `openmedia://auth/anilist/callback`
//! deep link completes the pending handshake (2-minute timeout, same as
//! upstream).
//!
//! Token storage note: upstream encrypted the token with Electron
//! `safeStorage` (OS keychain). This port stores it plaintext in
//! `openmedia.dat` alongside the other settings — acceptable for a personal
//! build, wire up Stronghold/keyutils before any public release.
//!
//! Client-id note: upstream used its own AniList API client ids. Register
//! your own app at anilist.co/settings/developer with redirect URL
//! `openmedia://auth/anilist/callback` and set `ANILIST_CLIENT_ID` below.

/// AniList API client id. Must have `openmedia://auth/anilist/callback`
/// registered as a redirect URI.
pub const ANILIST_CLIENT_ID: &str = "38904";

#[allow(dead_code)]
pub const APP_PROTOCOL: &str = "openmedia";
pub const ANILIST_OAUTH_REDIRECT_URI: &str = "openmedia://auth/anilist/callback";
#[allow(dead_code)]
pub const ANILIST_PIN_REDIRECT_URI: &str = "https://anilist.co/api/v2/oauth/pin";

const ACCESS_TOKEN_KEY: &str = "anilist.accessToken";
const AUTH_TIMEOUT_SECS: u64 = 120;

pub fn stored_token(app: &tauri::AppHandle) -> Option<String> {
    crate::store::get_store_str(app, ACCESS_TOKEN_KEY)
}

pub fn save_token(app: &tauri::AppHandle, token: &str) {
    let _ = crate::store::set_store_value(
        app,
        ACCESS_TOKEN_KEY,
        serde_json::json!(token),
    );
}

pub fn clear_token(app: &tauri::AppHandle, state: &super::state::AniListState) {
    let _ = crate::store::delete_store_key(app, ACCESS_TOKEN_KEY);
    state.clear_viewer_cache();
}

/// Extract `access_token` from query, `#fragment`, or raw regex fallback.
/// 1:1 with `parseAccessTokenFromUrl`.
pub fn parse_access_token_from_url(url: &str) -> Option<String> {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(token) = parsed
            .query_pairs()
            .find(|(k, _)| k == "access_token")
            .map(|(_, v)| v.into_owned())
        {
            if !token.is_empty() {
                return Some(token);
            }
        }
        let hash = parsed.fragment().unwrap_or("");
        if !hash.is_empty() {
            if let Some(token) = url::form_urlencoded::parse(hash.as_bytes())
                .find(|(k, _)| k == "access_token")
                .map(|(_, v)| v.into_owned())
            {
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
    }
    // Raw fallback for non-URL-shaped callbacks.
    let mut out: Option<String> = None;
    for part in url.split(['#', '&', '?']) {
        if let Some(v) = part.strip_prefix("access_token=") {
            out = percent_decode(v);
            break;
        }
    }
    out.filter(|s| !s.is_empty())
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn is_callback_url(url: &str) -> bool {
    url.starts_with(&format!("{ANILIST_OAUTH_REDIRECT_URI}?")) || url == ANILIST_OAUTH_REDIRECT_URI
}

fn authorize_url() -> String {
    format!(
        "https://anilist.co/api/v2/oauth/authorize?client_id={ANILIST_CLIENT_ID}&response_type=token&redirect_uri={}",
        url::form_urlencoded::byte_serialize(ANILIST_OAUTH_REDIRECT_URI.as_bytes()).collect::<String>()
    )
}

fn pin_authorize_url() -> String {
    format!(
        "https://anilist.co/api/v2/oauth/authorize?client_id={ANILIST_CLIENT_ID}&response_type=token"
    )
}

/// Complete a pending sign-in from a deep-link URL. No-op without a waiter
/// (mirrors upstream, including cold-start argv handling).
pub fn complete_oauth_from_deep_link(
    app: &tauri::AppHandle,
    state: &super::state::AniListState,
    url: &str,
) {
    if !is_callback_url(url) {
        return;
    }
    let sender = state.pending_auth.lock().unwrap().take();
    let Some(sender) = sender else { return };
    match parse_access_token_from_url(url) {
        Some(token) => {
            save_token(app, &token);
            let _ = sender.send(token);
        }
        None => {
            // Receiver derives its own error on drop; nothing to send.
            drop(sender);
        }
    }
}

/// Open the system browser and wait for the deep-link callback.
pub async fn connect_account(
    app: tauri::AppHandle,
    state: &super::state::AniListState,
) -> Result<serde_json::Value, String> {
    if state.pending_auth.lock().unwrap().is_some() {
        return Ok(serde_json::json!({ "ok": false, "error": "AniList sign-in is already in progress." }));
    }
    open_in_browser(&app, &authorize_url())?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    *state.pending_auth.lock().unwrap() = Some(tx);
    match tokio::time::timeout(std::time::Duration::from_secs(AUTH_TIMEOUT_SECS), rx).await {
        Ok(Ok(_)) => Ok(serde_json::json!({ "ok": true })),
        Ok(Err(_)) => {
            // Sender dropped without a token (callback had no access_token).
            Ok(serde_json::json!({ "ok": false, "error": "AniList callback did not include an access token." }))
        }
        Err(_) => {
            state.pending_auth.lock().unwrap().take();
            Ok(serde_json::json!({ "ok": false, "error": "AniList sign-in timed out." }))
        }
    }
}

fn open_in_browser(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

pub async fn open_pin_auth(app: &tauri::AppHandle) -> Result<(), String> {
    open_in_browser(app, &pin_authorize_url())
}

pub async fn submit_manual_token(
    state: &super::state::AniListState,
    app: &tauri::AppHandle,
    raw_token: &str,
) -> Result<serde_json::Value, String> {
    let token = raw_token.trim();
    if token.is_empty() {
        return Ok(serde_json::json!({ "ok": false, "error": "Paste an access token from the AniList pin page." }));
    }
    match fetch_viewer_name(&state.http, token).await {
        Some(_) => {
            save_token(app, token);
            Ok(serde_json::json!({ "ok": true }))
        }
        None => Ok(serde_json::json!({ "ok": false, "error": "That token is invalid or expired." })),
    }
}

pub async fn fetch_viewer_name(http: &reqwest::Client, token: &str) -> Option<String> {
    let data = super::api::post_graphql(
        http,
        "query { Viewer { id name } }",
        serde_json::json!({}),
        Some(token),
    )
    .await
    .ok()?;
    data.get("Viewer")
        .and_then(|v| v.get("name"))
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

pub async fn integration_status(
    state: &super::state::AniListState,
    app: &tauri::AppHandle,
) -> serde_json::Value {
    let Some(token) = stored_token(app) else {
        return serde_json::json!({ "connected": false });
    };
    match fetch_viewer_name(&state.http, &token).await {
        Some(username) => serde_json::json!({ "connected": true, "username": username }),
        None => {
            clear_token(app, state);
            serde_json::json!({ "connected": false })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_from_query() {
        assert_eq!(
            parse_access_token_from_url("openmedia://auth/anilist/callback?access_token=abc123"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn token_from_fragment() {
        assert_eq!(
            parse_access_token_from_url("openmedia://auth/anilist/callback#access_token=xyz&token_type=Bearer"),
            Some("xyz".to_string())
        );
    }

    #[test]
    fn token_percent_decoded() {
        assert_eq!(
            parse_access_token_from_url("openmedia://auth/cb?access_token=a%2Fb%20c"),
            Some("a/b c".to_string())
        );
    }

    #[test]
    fn token_missing() {
        assert_eq!(
            parse_access_token_from_url("openmedia://auth/anilist/callback?error=denied"),
            None
        );
    }
}
