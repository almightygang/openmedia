use tauri::Manager;

use crate::providers::{framework::StreamMode, Providers, ProviderContext};

fn providers(app: &tauri::AppHandle) -> tauri::State<'_, Providers> {
    app.state::<Providers>()
}

fn read_enabled_providers(app: &tauri::AppHandle) -> Vec<String> {
    let store = crate::store::read_store(app);
    if let Some(arr) = store.get("stream.providers").and_then(|v| v.as_array()) {
        let candidates: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let resolved = crate::providers::resolve_providers(&candidates);
        // Only trust the stored list when it actually contained a known provider.
        if candidates.iter().any(|c| crate::providers::is_known(c)) {
            return resolved;
        }
    }
    let stored = crate::store::get_store_str(app, "stream.provider");
    crate::providers::resolve_providers(
        &stored.into_iter().collect::<Vec<_>>(),
    )
}

fn resolve_name(app: &tauri::AppHandle, requested: Option<String>) -> String {
    let stored = crate::store::get_store_str(app, "stream.provider");
    let enabled = read_enabled_providers(app);
    if let Some(req) = requested.as_deref() {
        if crate::providers::is_known(req) {
            return req.to_string();
        }
    }
    if let Some(first) = enabled.first() {
        return first.clone();
    }
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
    // Keep the enabled set in sync so the legacy single-select migrates forward.
    let mut enabled = read_enabled_providers(&app);
    if !enabled.iter().any(|n| n == &resolved) {
        enabled.insert(0, resolved.clone());
        crate::store::set_store_value(&app, "stream.providers", serde_json::json!(enabled))?;
    }
    Ok(resolved)
}

#[tauri::command]
pub fn stream_provider_enabled_list(app: tauri::AppHandle) -> Vec<String> {
    read_enabled_providers(&app)
}

#[tauri::command]
pub async fn stream_provider_enabled_set(
    app: tauri::AppHandle,
    providers: Vec<String>,
) -> Result<Vec<String>, String> {
    let resolved = crate::providers::resolve_providers(&providers);
    if !providers.iter().any(|c| crate::providers::is_known(c)) {
        return Err("Enable at least one stream provider.".to_string());
    }
    crate::store::set_store_value(&app, "stream.providers", serde_json::json!(resolved))?;
    if let Some(first) = resolved.first() {
        crate::store::set_store_value(&app, "stream.provider", serde_json::json!(first))?;
    }
    Ok(resolved)
}

/// Tag every item in a provider result array with its source provider name.
fn tag_results(mut items: Vec<serde_json::Value>, provider: &str) -> Vec<serde_json::Value> {
    for item in &mut items {
        if let Some(obj) = item.as_object_mut() {
            obj.entry("provider").or_insert_with(|| serde_json::json!(provider));
        }
    }
    items
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
    let enabled = read_enabled_providers(&app);
    let state = providers(&app);
    let mut merged: Vec<serde_json::Value> = Vec::new();
    let mut first_error: Option<String> = None;
    for name in &enabled {
        let result = match name.as_str() {
            "animeparadise" => state.animeparadise.search(&query).await,
            "twitch" => state.twitch.search(&query).await,
            "youtube" => state.youtube.search(&query).await,
            other => Err(format!("Unknown stream provider: {other}")),
        };
        match result {
            Ok(value) => merged.extend(tag_results(as_array(value), name)),
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if merged.is_empty() {
        if let Some(e) = first_error {
            return Err(e);
        }
    }
    Ok(merged)
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
    provider_override: Option<String>,
) -> Result<Vec<String>, String> {
    let effective = resolve_name(&app, provider_override);
    if StreamMode::from_str(mode.as_deref()) == StreamMode::Dub
        && effective == "animeparadise"
    {
        return Ok(vec![]);
    }
    let state = providers(&app);
    Ok(as_strings(match effective.as_str() {
        "twitch" => state.twitch.get_episodes_list(&provider_id).await?,
        "youtube" => state.youtube.get_episodes_list(&provider_id).await?,
        _ => state.animeparadise.get_episodes_list(&provider_id).await?,
    }))
}

#[tauri::command]
pub async fn stream_provider_stream_url(
    app: tauri::AppHandle,
    _id: Option<String>,
    provider_id: Option<String>,
    episode: String,
    mode: Option<String>,
    provider_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let effective = resolve_name(&app, provider_override);
    let state = providers(&app);
    let mut value = match effective.as_str() {
        "twitch" => {
            state
                .twitch
                .get_stream_url(provider_id.as_deref(), &episode, StreamMode::from_str(mode.as_deref()))
                .await?
        }
        "youtube" => {
            state
                .youtube
                .get_stream_url(provider_id.as_deref(), &episode, StreamMode::from_str(mode.as_deref()))
                .await?
        }
        _ => {
            state
                .animeparadise
                .get_stream_url(provider_id.as_deref(), &episode, StreamMode::from_str(mode.as_deref()))
                .await?
        }
    };
    if let Some(obj) = value.as_object_mut() {
        obj.entry("provider").or_insert_with(|| serde_json::json!(effective));
    }
    Ok(value)
}

/// Re-resolve an already-open stream at a different quality without restarting
/// the session. Used for progressive (non-HLS) sources where the proxy
/// `?variant=` filter cannot switch renditions (each rendition is its own URL).
#[tauri::command]
pub async fn stream_provider_quality_url(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    episode: String,
    quality: String,
    provider_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let effective = resolve_name(&app, provider_override);
    let state = providers(&app);
    let video_ref = provider_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(episode.as_str());
    let mut value = match effective.as_str() {
        "twitch" => {
            // providerId carries the login; episode is just "live".
            let login = provider_id
                .as_deref()
                .filter(|s| {
                    let t = s.trim();
                    !t.is_empty() && t != "live" && t != "1"
                })
                .unwrap_or(episode.as_str());
            state.twitch.get_quality_url(login, &quality).await?
        }
        "youtube" => state.youtube.get_quality_url(video_ref, &quality).await?,
        _ => {
            return Err("Quality re-resolve is only supported for Twitch/YouTube".to_string());
        }
    };
    if let Some(obj) = value.as_object_mut() {
        obj.entry("provider").or_insert_with(|| serde_json::json!(effective));
    }
    Ok(value)
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
pub fn stream_provider_ffmpeg_status(
    state: tauri::State<'_, crate::proxy::ProxyState>,
) -> serde_json::Value {
    let path = state.ffmpeg_path.clone();
    serde_json::json!({
        "available": path.is_some(),
        "path": path,
    })
}

#[tauri::command]
pub fn stream_provider_transcode_last_error(
    state: tauri::State<'_, crate::proxy::ProxyState>,
    target_url: String,
    variant: Option<String>,
) -> serde_json::Value {
    let key = crate::proxy::hls::transcode_cache_key(&target_url, variant.as_deref());
    let (progress, stderr_tail) = state.with_inner(|i| {
        let progress = i
            .sessions
            .get(&key)
            .and_then(|s| s.sequential_job.as_ref())
            .map(|j| j.stderr_tail.clone());
        let progress_json = i.progress.get(&key).map(|p| p.to_json()).unwrap_or(
            serde_json::json!({
                "state": "idle",
                "progressPercent": null,
                "message": "Waiting to start...",
            }),
        );
        (progress_json, progress.unwrap_or_default())
    });
    serde_json::json!({
        "progress": progress,
        "stderrTail": stderr_tail.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>(),
    })
}

#[tauri::command]
pub async fn stream_provider_show_details(
    app: tauri::AppHandle,
    provider_id: String,
    provider_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let effective = resolve_name(&app, provider_override);
    let state = providers(&app);
    match effective.as_str() {
        "twitch" => state.twitch.get_show_details(&provider_id).await,
        "youtube" => state.youtube.get_show_details(&provider_id).await,
        _ => state.animeparadise.get_show_details(&provider_id).await,
    }
}

#[tauri::command]
pub async fn stream_provider_recent_uploads(
    app: tauri::AppHandle,
    page: u32,
    limit: Option<u32>,
    provider: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let enabled = match provider.as_deref().filter(|p| crate::providers::is_known(p)) {
        Some(single) => vec![single.to_string()],
        None => read_enabled_providers(&app),
    };
    let per_provider = limit.unwrap_or(12) as usize;
    let state = providers(&app);
    let mut merged: Vec<serde_json::Value> = Vec::new();
    let mut first_error: Option<String> = None;
    for name in &enabled {
        let result = match name.as_str() {
            "animeparadise" => {
                state
                    .animeparadise
                    .get_recent_uploads(page as i64, per_provider, StreamMode::Sub)
                    .await
            }
            "twitch" => {
                state
                    .twitch
                    .get_recent_uploads(page as i64, per_provider, StreamMode::Sub)
                    .await
            }
            "youtube" => {
                state
                    .youtube
                    .get_recent_uploads(page as i64, per_provider, StreamMode::Sub)
                    .await
            }
            other => Err(format!("Unknown stream provider: {other}")),
        };
        match result {
            Ok(value) => merged.extend(tag_results(as_array(value), name)),
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if merged.is_empty() {
        if let Some(e) = first_error {
            return Err(e);
        }
    } else if enabled.len() > 1 {
        merged.truncate(per_provider.max(1));
    }
    Ok(merged)
}

/// Build the shared provider registry (called once at startup).
pub fn build_providers() -> Providers {
    Providers::new(ProviderContext::new())
}
