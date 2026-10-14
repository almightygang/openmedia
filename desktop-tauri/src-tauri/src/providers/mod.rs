//! Stream providers — AnimeParadise only.
//!
//! The other six upstreams (HiAnime, AllAnime, AniDB, AnimePahe, ReAnime,
//! Senshi) were removed: none resolves a playable stream in this build
//! (Cloudflare gates, site migrations, dead crypto handshakes, or WASM-only
//! decryptors). See README for the per-provider verdicts.

pub mod animeparadise;
pub mod framework;

use std::collections::HashMap;
use std::sync::Mutex;

pub use framework::ProviderContext;

/// The sole provider, matching `streamProviders` in `stream-provider.ts`.
pub struct Providers {
    pub animeparadise: animeparadise::AnimeParadiseProvider,
}

impl Providers {
    pub fn new(ctx: ProviderContext) -> Self {
        Self {
            animeparadise: animeparadise::AnimeParadiseProvider::new(ctx),
        }
    }
}

pub fn is_known(name: &str) -> bool {
    name == "animeparadise"
}

pub fn resolve_provider(requested: Option<&str>, stored: Option<&str>) -> String {
    for candidate in [requested, stored].into_iter().flatten() {
        if is_known(candidate) {
            return candidate.to_string();
        }
    }
    "animeparadise".to_string()
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
