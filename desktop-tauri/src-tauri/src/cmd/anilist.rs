//! AniList Tauri commands — thin wrappers over `crate::anilist`.

use crate::anilist::AniListState;

#[tauri::command]
pub async fn anilist_show_details(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    media_id: i64,
) -> Result<serde_json::Value, String> {
    crate::anilist::media::show_details(&state, &app, media_id).await
}

#[tauri::command]
pub async fn anilist_search(
    state: tauri::State<'_, AniListState>,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    crate::anilist::media::fetch_media_page(&state, variables).await
}

#[tauri::command]
pub async fn anilist_popular_season(
    state: tauri::State<'_, AniListState>,
) -> Result<serde_json::Value, String> {
    crate::anilist::media::popular_season(&state).await
}

#[tauri::command]
pub async fn anilist_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
) -> Result<serde_json::Value, String> {
    crate::anilist::oauth::connect_account(app, &state).await
}

#[tauri::command]
pub fn anilist_disconnect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
) -> Result<(), String> {
    crate::anilist::oauth::clear_token(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn anilist_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
) -> Result<serde_json::Value, String> {
    Ok(crate::anilist::oauth::integration_status(&state, &app).await)
}

#[tauri::command]
pub async fn anilist_open_pin_auth(app: tauri::AppHandle) -> Result<(), String> {
    crate::anilist::oauth::open_pin_auth(&app).await
}

#[tauri::command]
pub async fn anilist_submit_manual_token(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    token: String,
) -> Result<serde_json::Value, String> {
    crate::anilist::oauth::submit_manual_token(&state, &app, &token).await
}

#[tauri::command]
pub async fn anilist_get_media_list(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    status: String,
    page: Option<i64>,
) -> Result<serde_json::Value, String> {
    crate::anilist::list::fetch_media_list(&state, &app, &status, page.unwrap_or(1)).await
}

#[tauri::command]
pub async fn anilist_get_favourites(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    page: Option<i64>,
) -> Result<serde_json::Value, String> {
    crate::anilist::favourites::fetch_favourites(&state, &app, page.unwrap_or(1)).await
}

#[tauri::command]
pub async fn anilist_save_list_entry(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    crate::anilist::list::save_entry(&state, &app, &input).await
}

#[tauri::command]
pub async fn anilist_delete_list_entry(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    list_entry_id: i64,
) -> Result<(), String> {
    crate::anilist::list::delete_entry(&state, &app, list_entry_id).await
}

#[tauri::command]
pub async fn anilist_toggle_favourite(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    media_id: i64,
) -> Result<bool, String> {
    crate::anilist::favourites::toggle_favourite(&state, &app, media_id).await
}

#[tauri::command]
pub async fn anilist_sync_watch_progress(
    app: tauri::AppHandle,
    state: tauri::State<'_, AniListState>,
    input: serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    if crate::cmd::privacy::is_incognito(&app) {
        return Ok(None);
    }
    Ok(Some(
        crate::anilist::list::sync_watch_progress(&state, &app, &input).await?,
    ))
}
