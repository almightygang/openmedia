//! Shared `openmedia.dat` JSON store helpers.
//!
//! The file lives in the Tauri app-data dir and holds plain settings
//! (`privacy.incognito`, `stream.provider`, `anilist.accessToken`, …).

use tauri::Manager;

fn store_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("openmedia.dat"))
}

pub fn read_store(app: &tauri::AppHandle) -> serde_json::Value {
    let path = match store_path(app) {
        Ok(p) => p,
        Err(_) => return serde_json::json!({}),
    };
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or(serde_json::json!({}))
}

pub fn write_store(app: &tauri::AppHandle, value: &serde_json::Value) -> Result<(), String> {
    let path = store_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_string(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn get_store_str(app: &tauri::AppHandle, key: &str) -> Option<String> {
    read_store(app)
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn set_store_value(app: &tauri::AppHandle, key: &str, value: serde_json::Value) -> Result<(), String> {
    let mut store = read_store(app);
    if !store.is_object() {
        store = serde_json::json!({});
    }
    store[key] = value;
    write_store(app, &store)
}

pub fn delete_store_key(app: &tauri::AppHandle, key: &str) -> Result<(), String> {
    let mut store = read_store(app);
    if let Some(obj) = store.as_object_mut() {
        obj.remove(key);
        write_store(app, &store)?;
    }
    Ok(())
}
