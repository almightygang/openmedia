//! Shared stream-proxy state: opaque upstream registry, HLS transcode
//! sessions, progress snapshots, and the outbound HTTP client.
//!
//! Ports the module-level maps in `main/stream-proxy.ts`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::oneshot;

use super::hls::upstream_target_id;

pub const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub const HLS_SEGMENT_DURATION_SECONDS: u64 = 6;
pub const SEGMENT_WAIT_TIMEOUT_MS: u64 = 45_000;
pub const SEGMENT_RESTART_FORWARD_TOLERANCE: u64 = 15;
pub const PROBE_TIMEOUT_MS: u64 = 15_000;
pub const PLAYLIST_PROBE_TIMEOUT_MS: u64 = 12_000;
pub const FIRST_SEGMENT_WAIT_MS: u64 = 45_000;
pub const TRANSCODE_CACHE_VERSION: &str = "v4";
pub const SAME_START_FAILURE_COOLDOWN_MS: u64 = 5000;
pub const ENSURE_SEGMENT_MAX_RESTART_RETRIES: u32 = 6;
const MAX_UPSTREAM_TARGETS: usize = 4096;

// ---------------------------------------------------------------------------
// upstream target registry (opaque ?u= ids)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UpstreamTarget {
    pub url: String,
    pub referer: Option<String>,
}

pub(crate) struct UpstreamRegistry {
    by_id: HashMap<String, UpstreamTarget>,
    id_by_key: HashMap<String, String>,
    order: VecDeque<String>,
}

impl UpstreamRegistry {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            id_by_key: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn register(&mut self, url: &str, referer: Option<&str>) -> String {
        let referer = referer.map(str::to_string);
        let key = format!("{}\0{}", url, referer.as_deref().unwrap_or(""));
        if let Some(existing) = self.id_by_key.get(&key) {
            if self.by_id.contains_key(existing) {
                return existing.clone();
            }
        }
        while self.by_id.len() >= MAX_UPSTREAM_TARGETS {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(target) = self.by_id.remove(&oldest) {
                let old_key = format!(
                    "{}\0{}",
                    target.url,
                    target.referer.as_deref().unwrap_or("")
                );
                if self.id_by_key.get(&old_key).map(|s| s.as_str()) == Some(oldest.as_str()) {
                    self.id_by_key.remove(&old_key);
                }
            }
        }
        let id = upstream_target_id(url, referer.as_deref());
        self.by_id.insert(
            id.clone(),
            UpstreamTarget {
                url: url.to_string(),
                referer,
            },
        );
        self.id_by_key.insert(key, id.clone());
        self.order.push_back(id.clone());
        id
    }

    fn resolve(&self, id: &str) -> Option<UpstreamTarget> {
        self.by_id.get(id).cloned()
    }
}

// ---------------------------------------------------------------------------
// transcode sessions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SegmentWaitError {
    PastEof(u32),
    TranscoderRestarted { seg: u32, new_start: u32 },
    Failed(String),
    Cancelled,
    TimedOut(u32),
    ClientAborted,
}

impl std::fmt::Display for SegmentWaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PastEof(seg) => write!(f, "Segment {seg} is past the end of the stream"),
            Self::TranscoderRestarted { seg, new_start } => write!(
                f,
                "Transcoder restarted at segment {new_start}; segment {seg} no longer scheduled"
            ),
            Self::Failed(msg) => write!(f, "{msg}"),
            Self::Cancelled => write!(f, "Transcode cancelled"),
            Self::TimedOut(seg) => write!(f, "Timed out waiting for segment {seg}"),
            Self::ClientAborted => write!(f, "Client aborted segment request"),
        }
    }
}

pub struct SequentialJob {
    pub child: tokio::process::Child,
    pub start_segment: u32,
    pub highest_completed_segment: i64,
    pub stopped: bool,
    pub stderr_tail: String,
    pub exit_code: Option<i32>,
    pub clean_exit: bool,
}

pub(crate) type AwaiterQueue = Vec<oneshot::Sender<Result<(), SegmentWaitError>>>;

#[allow(dead_code)]
pub struct HlsSession {
    pub key: String,
    pub input_url: String,
    pub target_url: String,
    pub referer: Option<String>,
    /// True when the playlist probe identified an HLS input (HLS-demuxer
    /// ffmpeg options are only valid then).
    pub hls_input: bool,
    pub duration_seconds: f64,
    pub segment_count: u32,
    pub segment_dir: std::path::PathBuf,
    pub sequential_job: Option<SequentialJob>,
    pub segments_available: std::collections::HashSet<u32>,
    pub segments_unavailable: std::collections::HashSet<u32>,
    pub awaiters: HashMap<u32, AwaiterQueue>,
    pub last_failed_start_segment: Option<u32>,
    pub last_failed_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct TranscodeProgress {
    pub state: &'static str,
    pub progress_percent: Option<f64>,
    pub message: String,
}

impl TranscodeProgress {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state,
            "progressPercent": self.progress_percent,
            "message": self.message,
        })
    }
}

pub(crate) struct Inner {
    pub(crate) upstream: UpstreamRegistry,
    pub(crate) sessions: HashMap<String, HlsSession>,
    pub(crate) session_inits: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    pub(crate) progress: HashMap<String, TranscodeProgress>,
    pub(crate) port: u16,
}

#[derive(Clone)]
pub struct ProxyState {
    inner: Arc<Mutex<Inner>>,
    pub http: reqwest::Client,
    pub ffmpeg_path: Option<String>,
    pub cache_dir: std::path::PathBuf,
}

impl ProxyState {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_ffmpeg(None)
    }

    pub fn with_ffmpeg(ffmpeg_path: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .cookie_store(true)
            .user_agent(CHROME_UA)
            .build()
            .expect("reqwest client");
        let cache_dir = std::env::temp_dir()
            .join("openanime-transcode-cache")
            .join(TRANSCODE_CACHE_VERSION);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                upstream: UpstreamRegistry::new(),
                sessions: HashMap::new(),
                session_inits: HashMap::new(),
                progress: HashMap::new(),
                port: 0,
            })),
            http,
            ffmpeg_path,
            cache_dir,
        }
    }

    pub fn set_port(&self, port: u16) {
        self.inner.lock().unwrap().port = port;
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.inner.lock().unwrap().port)
    }

    pub fn register_upstream(&self, url: &str, referer: Option<&str>) -> String {
        self.inner.lock().unwrap().upstream.register(url, referer)
    }

    pub fn resolve_upstream(&self, id: &str) -> Option<UpstreamTarget> {
        self.inner.lock().unwrap().upstream.resolve(id)
    }

    pub(crate) fn with_inner<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.inner.lock().unwrap())
    }

    pub fn progress_json(&self, key: &str) -> serde_json::Value {
        let inner = self.inner.lock().unwrap();
        match inner.progress.get(key) {
            Some(p) => p.to_json(),
            None => serde_json::json!({
                "state": "idle",
                "progressPercent": null,
                "message": "Waiting to start...",
            }),
        }
    }

    pub fn set_progress(&self, key: &str, progress: TranscodeProgress) {
        self.inner.lock().unwrap().progress.insert(key.to_string(), progress);
    }
}
