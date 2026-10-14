//! In-memory AniList state: HTTP client, viewer-id cache,
//! popular-season cache, and the pending OAuth handshake.

use std::sync::Mutex;
use std::time::Instant;

use tokio::sync::oneshot;

pub struct SeasonCache {
    pub season_key: String,
    pub fetched_at: Instant,
    pub data: serde_json::Value,
}

pub struct AniListState {
    pub http: reqwest::Client,
    pub viewer_id: Mutex<Option<i64>>,
    pub season_cache: Mutex<Option<SeasonCache>>,
    pub pending_auth: Mutex<Option<oneshot::Sender<String>>>,
}

impl AniListState {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(crate::proxy::state::CHROME_UA)
                .build()
                .expect("anilist reqwest client"),
            viewer_id: Mutex::new(None),
            season_cache: Mutex::new(None),
            pending_auth: Mutex::new(None),
        }
    }

    pub fn clear_viewer_cache(&self) {
        *self.viewer_id.lock().unwrap() = None;
    }
}
