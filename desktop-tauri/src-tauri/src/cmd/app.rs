use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppUpdateCheckResult {
    #[serde(rename = "updateAvailable")]
    pub update_available: bool,
    pub version: Option<String>,
    #[serde(rename = "releaseUrl")]
    pub release_url: Option<String>,
}

#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub fn app_os() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
pub async fn app_check_for_update(
    app: tauri::AppHandle,
) -> Result<AppUpdateCheckResult, String> {
    // Phase 1 stub: report current version, no remote check yet.
    // Full implementation uses tauri-plugin-updater against GitHub releases.
    let current = app.package_info().version.to_string();
    Ok(AppUpdateCheckResult {
        update_available: false,
        version: Some(current),
        release_url: None,
    })
}

#[tauri::command]
pub fn app_open_logs_directory(app: tauri::AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open::that(dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn app_list_github_issues() -> Result<serde_json::Value, String> {
    // TODO: port list-github-issues.ts (public GitHub API, no auth).
    Ok(serde_json::json!({ "issues": [] }))
}

#[tauri::command]
pub async fn app_list_pinned_github_issues() -> Result<serde_json::Value, String> {
    // TODO: port list-pinned-github-issues.ts.
    Ok(serde_json::json!({ "issues": [] }))
}

// `open` crate is intentionally not a dependency yet; resolve via opener plugin on frontend.
// Keep this helper local to avoid adding deps in scaffold phase.
mod open {
    #![allow(clippy::needless_return)]
    pub fn that<P: AsRef<std::path::Path>>(path: P) -> Result<(), String> {
        let p = path.as_ref().to_string_lossy().to_string();
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg(&p)
                .spawn()
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(&p)
                .spawn()
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = p;
            return Err("unsupported platform".into());
        }
    }
}
