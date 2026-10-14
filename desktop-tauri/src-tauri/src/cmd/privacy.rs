use std::path::PathBuf;
use tauri::Manager;

fn store_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("openmedia.dat"))
}

fn read_store(app: &tauri::AppHandle) -> serde_json::Value {
    let path = match store_path(app) {
        Ok(p) => p,
        Err(_) => return serde_json::Value::Null,
    };
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null)
}

fn write_store(app: &tauri::AppHandle, value: &serde_json::Value) -> Result<(), String> {
    let path = store_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_string(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn privacy_incognito_get(app: tauri::AppHandle) -> bool {
    read_store(&app)
        .get("privacy.incognito")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

#[tauri::command]
pub fn privacy_incognito_set(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let mut store = read_store(&app);
    if !store.is_object() {
        store = serde_json::json!({});
    }
    store["privacy.incognito"] = serde_json::json!(enabled);
    write_store(&app, &store)?;
    Ok(enabled)
}

pub fn is_incognito(app: &tauri::AppHandle) -> bool {
    privacy_incognito_get(app.clone())
}
