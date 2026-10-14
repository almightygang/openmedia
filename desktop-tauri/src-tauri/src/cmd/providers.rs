use tauri::Manager;

use crate::providers::{framework::StreamMode, Providers, ProviderContext};

fn providers(app: &tauri::AppHandle) -> tauri::State<'_, Providers> {
    app.state::<Providers>()
}

fn resolve_name(app: &tauri::AppHandle, requested: Option<String>) -> String {
    let stored = crate::store::get_store_str(app, "stream.provider");
    crate::providers::resolve_provider(requested.as_deref(), stored.as_deref())
}

#[tauri::command]
pub fn stream_provider_active_get(app: tauri::AppHandle) -> String {
    resolve_name(&app, None)
}

#[tauri::command]
pub async fn stream_provider_active_set(
    app: tauri::AppHandle,
    provider: String,
) -> Result<String, String> {
    let resolved = resolve_name(&app, Some(provider));
    crate::store::set_store_value(&app, "stream.provider", serde_json::json!(resolved))?;
    Ok(resolved)
}

fn as_array(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        _ => vec![],
    }
}

fn as_strings(value: serde_json::Value) -> Vec<String> {
    as_array(value)
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

#[tauri::command]
pub async fn stream_provider_search(
    app: tauri::AppHandle,
    query: String,
    _filters: Option<serde_json::Value>,
) -> Result<Vec<serde_json::Value>, String> {
    let _ = resolve_name(&app, None);
    Ok(as_array(providers(&app).animeparadise.search(&query).await?))
}

#[tauri::command]
pub async fn stream_provider_search_filters(
    _app: tauri::AppHandle,
) -> Result<Vec<serde_json::Value>, String> {
    Ok(vec![])
}

#[tauri::command]
pub async fn stream_provider_episodes(
    app: tauri::AppHandle,
    provider_id: String,
    mode: Option<String>,
    _provider_override: Option<String>,
) -> Result<Vec<String>, String> {
    let _ = resolve_name(&app, None);
    if StreamMode::from_str(mode.as_deref()) == StreamMode::Dub {
        return Ok(vec![]);
    }
    Ok(as_strings(
        providers(&app).animeparadise.get_episodes_list(&provider_id).await?,
    ))
}

#[tauri::command]
pub async fn stream_provider_stream_url(
    app: tauri::AppHandle,
    _id: Option<String>,
    provider_id: Option<String>,
    episode: String,
    mode: Option<String>,
    _provider_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = resolve_name(&app, None);
    providers(&app)
        .animeparadise
        .get_stream_url(provider_id.as_deref(), &episode, StreamMode::from_str(mode.as_deref()))
        .await
}

#[tauri::command]
pub fn stream_provider_proxy_base(state: tauri::State<'_, crate::proxy::ProxyState>) -> String {
    state.base_url()
}

#[tauri::command]
pub async fn stream_provider_prepare_transcode(
    state: tauri::State<'_, crate::proxy::ProxyState>,
    target_url: String,
    referer: Option<String>,
    variant: Option<String>,
) -> Result<bool, String> {
    let input_url = build_proxy_input_url(&state, &target_url, referer.as_deref(), variant.as_deref());
    crate::proxy::transcode::prepare_transcoded_stream(
        &state,
        &input_url,
        &target_url,
        referer.as_deref(),
        variant.as_deref(),
    )
    .await?;
    Ok(true)
}

fn build_proxy_input_url(
    state: &crate::proxy::ProxyState,
    target_url: &str,
    referer: Option<&str>,
    variant: Option<&str>,
) -> String {
    let id = state.register_upstream(target_url, referer);
    let mut query = format!("u={id}");
    if let Some(v) = variant.map(str::trim).filter(|s| !s.is_empty()) {
        query.push_str(&format!(
            "&variant={}",
            url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
        ));
    }
    format!("{}/stream/playlist.m3u8?{query}", state.base_url())
}

#[tauri::command]
pub fn stream_provider_cancel_transcode(
    state: tauri::State<'_, crate::proxy::ProxyState>,
    target_url: String,
    variant: Option<String>,
) -> Result<bool, String> {
    crate::proxy::transcode::cancel_transcoded_stream(&state, &target_url, variant.as_deref());
    Ok(true)
}

#[tauri::command]
pub fn stream_provider_transcode_progress(
    state: tauri::State<'_, crate::proxy::ProxyState>,
    target_url: String,
    variant: Option<String>,
) -> serde_json::Value {
    let key = crate::proxy::hls::transcode_cache_key(&target_url, variant.as_deref());
    state.progress_json(&key)
}

#[tauri::command]
pub async fn stream_provider_show_details(
    app: tauri::AppHandle,
    provider_id: String,
    _provider_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = resolve_name(&app, None);
    providers(&app).animeparadise.get_show_details(&provider_id).await
}

#[tauri::command]
pub async fn stream_provider_recent_uploads(
    app: tauri::AppHandle,
    page: u32,
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, String> {
    let _ = resolve_name(&app, None);
    Ok(as_array(
        providers(&app)
            .animeparadise
            .get_recent_uploads(page as i64, limit.unwrap_or(12) as usize, StreamMode::Sub)
            .await?,
    ))
}

/// Build the shared provider registry (called once at startup).
pub fn build_providers() -> Providers {
    Providers::new(ProviderContext::new())
}
