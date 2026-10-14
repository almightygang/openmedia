//! AniList GraphQL transport — port of `anilist-api.ts` and
//! `anilist-authed-api.ts`.

use serde_json::Value;

use super::state::AniListState;

pub const ANILIST_GRAPHQL_API: &str = "https://graphql.anilist.co";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotConnectedError;

impl std::fmt::Display for NotConnectedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AniList account is not connected.")
    }
}

/// POST a GraphQL operation. Mirrors `postAniListGraphql`: non-2xx and
/// GraphQL `errors` both throw; missing `data` throws.
pub async fn post_graphql(
    http: &reqwest::Client,
    query: &str,
    variables: Value,
    token: Option<&str>,
) -> Result<Value, String> {
    let mut req = http
        .post(ANILIST_GRAPHQL_API)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "query": query, "variables": variables }));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let res = req.send().await.map_err(|e| format!("AniList API error: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("AniList API error: {status}"));
    }
    let json: Value = res
        .json()
        .await
        .map_err(|e| format!("AniList API error: {e}"))?;
    if let Some(errors) = json.get("errors").and_then(|e| e.as_array()) {
        if let Some(first) = errors.first() {
            let msg = first
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("AniList query failed");
            return Err(msg.to_string());
        }
    }
    json.get("data")
        .cloned()
        .ok_or_else(|| "AniList response missing data".to_string())
}

/// Authed POST using the stored token. Mirrors `postAniListAuthedGraphql`.
pub async fn post_authed_graphql(
    state: &AniListState,
    app: &tauri::AppHandle,
    query: &str,
    variables: Value,
) -> Result<Value, String> {
    let token = super::oauth::stored_token(app).ok_or_else(|| NotConnectedError.to_string())?;
    post_graphql(&state.http, query, variables, Some(&token)).await
}

/// Cached viewer id. Mirrors `getAniListViewerId`.
pub async fn viewer_id(
    state: &AniListState,
    app: &tauri::AppHandle,
) -> Result<i64, String> {
    if let Some(id) = *state.viewer_id.lock().unwrap() {
        return Ok(id);
    }
    let data =
        post_authed_graphql(state, app, "query { Viewer { id name } }", serde_json::json!({})).await?;
    let id = data
        .get("Viewer")
        .and_then(|v| v.get("id"))
        .and_then(|id| id.as_i64())
        .filter(|id| *id > 0)
        .ok_or_else(|| "Failed to resolve AniList viewer id.".to_string())?;
    *state.viewer_id.lock().unwrap() = Some(id);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn graphql_errors_surface_first_message() {
        // `id: "x"` is a validation error on AniList's side — exercises the
        // errors-envelope path against the live API.
        let http = reqwest::Client::new();
        let err = post_graphql(
            &http,
            "query Media($id: Int) { Media(id: $id) { id } }",
            serde_json::json!({"id": "not-an-int"}),
            None,
        )
        .await
        .expect_err("expected a GraphQL validation error");
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn live_public_media_query() {
        // Cowboy Bebop's stable id — exercises the happy path.
        let http = reqwest::Client::new();
        let data = post_graphql(
            &http,
            "query Media($id: Int) { Media(id: $id) { id title { userPreferred } } }",
            serde_json::json!({"id": 1}),
            None,
        )
        .await
        .expect("live AniList query");
        assert_eq!(data.get("Media").and_then(|m| m.get("id")), Some(&serde_json::json!(1)));
    }
}
