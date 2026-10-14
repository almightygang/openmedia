//! ffmpeg sidecar resolution — port of `resolveFfmpegPath()` in
//! `main/stream-proxy.ts`, adapted to Tauri `externalBin` sidecars.
//!
//! The sidecar binaries (`ffmpeg-x86_64-unknown-linux-gnu`,
//! `ffmpeg-x86_64-pc-windows-msvc.exe`) are downloaded by
//! `scripts/download-ffmpeg.mjs` and bundled next to the app binary.

use std::path::PathBuf;

fn current_triple_sidecar_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg-x86_64-pc-windows-msvc.exe"
    } else {
        "ffmpeg-x86_64-unknown-linux-gnu"
    }
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        path.is_file()
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && std::fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }
}

pub fn resolve_ffmpeg_path(app: &tauri::AppHandle) -> Option<String> {
    use tauri::Manager;
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Tauri externalBin sidecars ship next to the app executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("ffmpeg"));
            candidates.push(dir.join("ffmpeg.exe"));
            candidates.push(dir.join(current_triple_sidecar_name()));
        }
    }
    // Tauri resource dir fallback.
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("ffmpeg"));
        candidates.push(resource_dir.join("ffmpeg.exe"));
        candidates.push(resource_dir.join(current_triple_sidecar_name()));
        candidates.push(resource_dir.join("binaries").join(current_triple_sidecar_name()));
    }
    // Dev fallback: src-tauri/binaries next to the crate manifest.
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(current_triple_sidecar_name()),
    );
    // System PATH last.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            #[cfg(target_os = "windows")]
            candidates.push(dir.join("ffmpeg.exe"));
            #[cfg(not(target_os = "windows"))]
            candidates.push(dir.join("ffmpeg"));
        }
    }

    candidates
        .into_iter()
        .find(|p| is_executable(p))
        .map(|p| p.to_string_lossy().into_owned())
}
