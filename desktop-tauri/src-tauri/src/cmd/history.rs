use tauri::Manager;

use super::privacy::is_incognito;

fn history_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("watch-history.json"))
}

fn read_entries(app: &tauri::AppHandle) -> Vec<serde_json::Value> {
    let path = match history_path(app) {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    match parsed {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter(|e| {
                e.get("id").and_then(|v| v.as_str()).is_some()
                    && e.get("provider").and_then(|v| v.as_str()).is_some()
            })
            .collect(),
        _ => vec![],
    }
}

fn write_entries(app: &tauri::AppHandle, entries: &[serde_json::Value]) -> Result<(), String> {
    let path = history_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_string(entries).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recently_watched_upsert(
    app: tauri::AppHandle,
    entry: serde_json::Value,
) -> Result<(), String> {
    if is_incognito(&app) {
        return Ok(());
    }
    let id = entry
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if id.is_empty() {
        return Ok(());
    }
    let mut entries = read_entries(&app);
    entries.retain(|e| e.get("id").and_then(|v| v.as_str()) != Some(id.as_str()));
    entries.push(entry);
    write_entries(&app, &entries)
}

#[tauri::command]
pub fn recently_watched_read(app: tauri::AppHandle) -> Vec<serde_json::Value> {
    read_entries(&app)
}

#[tauri::command]
pub fn recently_watched_clear(app: tauri::AppHandle) -> Result<(), String> {
    write_entries(&app, &[])
}

#[tauri::command]
pub fn recently_watched_remove(app: tauri::AppHandle, id: String) -> Result<(), String> {
    if id.is_empty() {
        return Ok(());
    }
    let entries = read_entries(&app);
    let next: Vec<_> = entries
        .into_iter()
        .filter(|e| e.get("id").and_then(|v| v.as_str()) != Some(id.as_str()))
        .collect();
    write_entries(&app, &next)
}
