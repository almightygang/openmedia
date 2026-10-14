//! Stream providers — AnimeParadise + live (Twitch GQL-only, YouTube InnerTube).
//!
//! AnimeParadise remains the only anime VOD upstream (the other six were
//! removed: Cloudflare gates, site migrations, dead crypto, WASM-only
//! decryptors). Twitch resolves anonymously via public GQL + Usher (no
//! developer registration); YouTube via anonymous InnerTube player.
//! Kick is intentionally skipped (home page stays "coming soon").

pub mod animeparadise;
pub mod framework;
pub mod hls;
pub mod twitch;
pub mod youtube;

use std::collections::HashMap;
use std::sync::Mutex;

pub use framework::ProviderContext;

/// All known provider names in priority order.
pub const KNOWN_PROVIDERS: &[&str] = &["animeparadise", "twitch", "youtube"];

/// The provider registry, matching `streamProviders` in `stream-provider.ts`.
pub struct Providers {
    pub animeparadise: animeparadise::AnimeParadiseProvider,
    pub twitch: twitch::TwitchProvider,
    pub youtube: youtube::YouTubeProvider,
}

impl Providers {
    pub fn new(ctx: ProviderContext) -> Self {
        Self {
            animeparadise: animeparadise::AnimeParadiseProvider::new(ctx.clone()),
            twitch: twitch::TwitchProvider::new(ctx.clone()),
            youtube: youtube::YouTubeProvider::new(ctx),
        }
    }
}

pub fn is_known(name: &str) -> bool {
    KNOWN_PROVIDERS.contains(&name)
}

pub fn resolve_provider(requested: Option<&str>, stored: Option<&str>) -> String {
    for candidate in [requested, stored].into_iter().flatten() {
        if is_known(candidate) {
            return candidate.to_string();
        }
    }
    "animeparadise".to_string()
}

/// Resolve the full enabled set, preserving priority order.
/// Unknown names are dropped; empty input falls back to the default provider.
pub fn resolve_providers(candidates: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in candidates {
        if is_known(name.as_str()) && !out.iter().any(|n| n == name) {
            out.push(name.clone());
        }
    }
    if out.is_empty() {
        out.push("animeparadise".to_string());
    }
    out
}

/// In-memory per-key TTL cache shared by providers.
pub struct TtlCache {
    map: Mutex<HashMap<String, (std::time::Instant, serde_json::Value)>>,
    ttl: std::time::Duration,
}

impl TtlCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            ttl: std::time::Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        let mut map = self.map.lock().unwrap();
        let (at, value) = map.get(key)?;
        if at.elapsed() > self.ttl {
            map.remove(key);
            return None;
        }
        Some(value.clone())
    }

    pub fn set(&self, key: &str, value: serde_json::Value) {
        self.map
            .lock()
            .unwrap()
            .insert(key.to_string(), (std::time::Instant::now(), value));
    }
}
