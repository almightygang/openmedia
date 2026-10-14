#[tauri::command]
pub fn theme_current() -> String {
    // TODO: persist per-window theme; default matches old main.ts (dark).
    "dark".to_string()
}

#[tauri::command]
pub fn theme_toggle() -> bool {
    // TODO: track state; report dark-mode = true for now.
    true
}

#[tauri::command]
pub fn theme_dark() {}

#[tauri::command]
pub fn theme_light() {}

#[tauri::command]
pub fn theme_system() -> bool {
    true
}
