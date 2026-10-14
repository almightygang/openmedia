//! Twitch provider — anonymous GQL-only (Option 2).
//!
//! No developer registration, no client secret, no user login. Uses Twitch's
//! public web client id against `gql.twitch.tv/gql` for metadata and the
//! `PlaybackAccessToken` persisted query + Usher for HLS playback, the same
//! approach community players use. Content only: live streams, no chat.

use serde_json::{json, Value};

use super::framework::{as_str, ProviderContext, StreamMode};
use super::hls;
use super::TtlCache;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// Twitch web client id (public, ships in the site itself).
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
/// Persisted query hash for `PlaybackAccessToken`. If Twitch rotates it,
/// playback fails closed with a `PersistedQueryNotFound` error and this
/// constant needs bumping.
const PLAYBACK_HASH: &str = "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:146.0) Gecko/20100101 Firefox/146.0";
const REFERER: &str = "https://player.twitch.tv";
const SEARCH_LIMIT: usize = 12;

const USER_QUERY: &str = "query($login:String!){user(login:$login){id login displayName profileImageURL(width:300) stream{id title viewersCount createdAt game{id displayName} previewImageURL} broadcastSettings{title game{id displayName}}}}";

pub struct TwitchProvider {
    ctx: ProviderContext,
    meta: TtlCache,
    masters: TtlCache,
}

impl TwitchProvider {
    pub fn new(ctx: ProviderContext) -> Self {
        Self {
            ctx,
            meta: TtlCache::new(60),
            masters: TtlCache::new(110),
        }
    }

    fn normalize_login(s: &str) -> String {
        s.trim().trim_start_matches('@').to_lowercase()
    }

    async fn gql(&self, body: Value) -> Result<Value, String> {
        let res = self
            .ctx
            .http
            .post(GQL_URL)
            .header("Client-ID", WEB_CLIENT_ID)
            .header("User-Agent", UA)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Twitch request failed: {e}"))?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("Twitch request failed ({status}): {GQL_URL}"));
        }
        res.json::<Value>()
            .await
            .map_err(|e| format!("Twitch read failed: {e}"))
    }

    async fn fetch_user(&self, login: &str) -> Result<Value, String> {
        let login = Self::normalize_login(login);
        if login.is_empty() {
            return Err("Empty Twitch channel".to_string());
        }
        if let Some(cached) = self.meta.get(&login) {
            return Ok(cached);
        }
        let body = json!({
            "query": USER_QUERY,
            "variables": { "login": login },
        });
        let v = self.gql(body).await?;
        if let Some(errs) = v.get("errors").and_then(|e| e.as_array()) {
            if !errs.is_empty() {
                let msg = errs
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!("Twitch query failed: {msg}"));
            }
        }
        let user = v.pointer("/data/user").cloned().unwrap_or(Value::Null);
        if user.is_null() {
            return Err(format!("Twitch channel '{login}' not found"));
        }
        self.meta.set(&login, user.clone());
        Ok(user)
    }

    fn is_live(user: &Value) -> bool {
        user.get("stream").map(|s| !s.is_null()).unwrap_or(false)
    }

    /// Twitch serves preview thumbnails as templates
    /// (`…live_user_{login}-{width}x{height}.jpg`); fill in a concrete size.
    /// Falls back to the static preview URL when the template is absent.
    fn preview(login: &str, template: Option<String>) -> String {
        match template {
            Some(t) if !t.is_empty() => t
                .replace("{width}", "640")
                .replace("%{width}", "640")
                .replace("{height}", "360")
                .replace("%{height}", "360"),
            _ => format!("https://static-cdn.jtvnw.net/previews-ttv/live_user_{login}-640x360.jpg"),
        }
    }

    fn row_from_user(&self, user: &Value) -> Option<Value> {
        let login = as_str(user.get("login"))?.to_lowercase();
        let display = as_str(user.get("displayName")).unwrap_or_else(|| login.clone());
        let avatar = as_str(user.get("profileImageURL"));
        let stream = user.get("stream");
        let bcast = user.get("broadcastSettings");
        let (title, viewers, category, thumb, started) = match stream {
            Some(s) if !s.is_null() => (
                as_str(s.get("title"))
                    .or_else(|| as_str(bcast.and_then(|b| b.get("title"))))
                    .unwrap_or_default(),
                s.get("viewersCount").and_then(|v| v.as_u64()).unwrap_or(0),
                as_str(s.get("game").and_then(|g| g.get("displayName")))
                    .or_else(|| as_str(bcast.and_then(|b| b.get("game")).and_then(|g| g.get("displayName"))))
                    .unwrap_or_default(),
                Self::preview(&login, as_str(s.get("previewImageURL"))),
                as_str(s.get("createdAt")).unwrap_or_default(),
            ),
            _ => return None,
        };
        Some(json!({
            "id": format!("twitch:{login}"),
            "providerId": login,
            "title": { "english": format!("{display} — {title}"), "romanji": display, "native": title },
            "thumbnail": thumb,
            "availableEpisodes": { "sub": 1 },
            "viewerCount": viewers,
            "views": viewers,
            "isLive": true,
            "category": category,
            "channelName": display,
            "startedAt": started,
            "profileImage": avatar,
            "type": "LIVE",
            "status": "LIVE",
        }))
    }

    #[allow(dead_code)]
    fn episode_row(&self, login: &str, user: &Value) -> Option<Value> {
        let row = self.row_from_user(user)?;
        Some(json!({
            "id": format!("twitch:{login}"),
            "providerId": login,
            "provider": "twitch",
            "title": row["title"],
            "thumbnail": row["thumbnail"],
            "index": 1,
            "token": "live",
            "mode": "sub",
            "viewerCount": row["viewerCount"],
            "views": row["views"],
            "isLive": true,
            "category": row["category"],
        }))
    }

    pub async fn search(&self, query: &str) -> Result<Value, String> {
        let q = query.trim();
        if q.is_empty() {
            return self.get_recent_uploads(1, SEARCH_LIMIT, StreamMode::Sub).await;
        }
        // Exact channel lookup covers the common case with one request.
        let login = Self::normalize_login(q).replace(' ', "");
        match self.fetch_user(&login).await {
            Ok(user) => {
                if Self::is_live(&user) {
                    if let Some(row) = self.row_from_user(&user) {
                        return Ok(json!([row]));
                    }
                }
                // Channel exists but offline → empty, not an error.
                return Ok(json!([]));
            }
            Err(_) => {}
        }
        // Fallback: GQL channel search (best-effort, schema-tolerant).
        let body = json!({
            "query": "query($q:String!){searchChannels(query:$q,first:10){edges{node{id login displayName profileImageURL(width:300) broadcastSettings{title game{id displayName}} stream{id title viewersCount createdAt game{id displayName} previewImageURL}}}}}",
            "variables": { "q": q },
        });
        let v = match self.gql(body).await {
            Ok(v) => v,
            Err(_) => return Ok(json!([])),
        };
        let edges = v
            .pointer("/data/searchChannels/edges")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for e in edges.iter().take(SEARCH_LIMIT) {
            let node = e.get("node").unwrap_or(e);
            if let Some(row) = self.row_from_user(node) {
                out.push(row);
            }
        }
        Ok(json!(out))
    }

    pub async fn get_recent_uploads(
        &self,
        _page: i64,
        limit: usize,
        mode: StreamMode,
    ) -> Result<Value, String> {
        if mode == StreamMode::Dub {
            return Ok(json!([]));
        }
        // GQL directory: top live streams. Tolerant to shape drift.
        let body = json!({
            "query": "query($first:Int!){streams(first:$first){edges{node{id title viewersCount createdAt broadcaster{login displayName profileImageURL(width:300)} game{id displayName} previewImageURL}}}}",
            "variables": { "first": limit.clamp(1, 30) as i64 },
        });
        let v = self.gql(body).await?;
        let edges = v
            .pointer("/data/streams/edges")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for e in edges {
            let n = e.get("node").unwrap_or(&e);
            let login = n
                .get("broadcaster")
                .and_then(|b| as_str(b.get("login")))
                .or_else(|| as_str(n.get("userLogin")))
                .unwrap_or_default()
                .to_lowercase();
            if login.is_empty() {
                continue;
            }
            let display = n
                .get("broadcaster")
                .and_then(|b| as_str(b.get("displayName")))
                .unwrap_or_else(|| login.clone());
            let viewers = n.get("viewersCount").and_then(|v| v.as_u64()).unwrap_or(0);
            let title = as_str(n.get("title")).unwrap_or_default();
            let category = as_str(n.get("game").and_then(|g| g.get("displayName"))).unwrap_or_default();
            let thumb = Self::preview(&login, as_str(n.get("previewImageURL")));
            out.push(json!({
                "id": format!("twitch:{login}"),
                "providerId": login,
                "provider": "twitch",
                "title": { "english": format!("{display} — {title}"), "romanji": display, "native": title },
                "thumbnail": thumb,
                "index": 1,
                "token": "live",
                "mode": "sub",
                "viewerCount": viewers,
                "views": viewers,
                "isLive": true,
                "category": category,
            }));
        }
        Ok(json!(out))
    }

    pub async fn get_episodes_list(&self, provider_id: &str) -> Result<Value, String> {
        let user = self.fetch_user(provider_id).await?;
        if Self::is_live(&user) {
            Ok(json!(["live"]))
        } else {
            let login = Self::normalize_login(provider_id);
            Err(format!("{login} isn't live right now"))
        }
    }

    pub async fn get_show_details(&self, provider_id: &str) -> Result<Value, String> {
        let user = self.fetch_user(provider_id).await?;
        let login = as_str(user.get("login"))
            .unwrap_or_else(|| Self::normalize_login(provider_id));
        let display = as_str(user.get("displayName")).unwrap_or_else(|| login.clone());
        let avatar = as_str(user.get("profileImageURL"));
        let stream = user.get("stream");
        let title = stream
            .and_then(|s| as_str(s.get("title")))
            .or_else(|| user.get("broadcastSettings").and_then(|b| as_str(b.get("title"))))
            .unwrap_or_else(|| display.clone());
        let viewers = stream.and_then(|s| s.get("viewersCount")).and_then(|v| v.as_u64()).unwrap_or(0);
        let category = stream
            .and_then(|s| s.get("game").and_then(|g| as_str(g.get("displayName"))))
            .unwrap_or_default();
        let thumb = Self::preview(
            &login,
            stream.and_then(|s| as_str(s.get("previewImageURL"))),
        );
        Ok(json!({
            "id": format!("twitch:{login}"),
            "providerId": login,
            "name": display,
            "thumbnail": thumb,
            "profileImage": avatar,
            "type": "LIVE",
            "description": title,
            "viewerCount": viewers,
            "views": viewers,
            "isLive": !stream.map(|s| s.is_null()).unwrap_or(true),
            "category": category,
        }))
    }

    async fn playback_token(&self, login: &str) -> Result<(String, String), String> {
        let body = json!({
            "operationName": "PlaybackAccessToken",
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": PLAYBACK_HASH } },
            "variables": {
                "isLive": true,
                "login": login,
                "isVod": false,
                "vodID": "",
                "playerType": "embed",
                "platform": "site",
            },
        });
        let v = self.gql(body).await?;
        if v.get("errors").and_then(|e| e.as_array()).map(|a| !a.is_empty()).unwrap_or(false) {
            let msg = v["errors"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            if msg.contains("PersistedQueryNotFound") {
                return Err("Twitch rotated its playback query (PersistedQueryNotFound); update PLAYBACK_HASH".to_string());
            }
            return Err(format!("Twitch playback token failed: {msg}"));
        }
        let pat = v.pointer("/data/streamPlaybackAccessToken").ok_or_else(|| {
            format!("Twitch returned no playback token for '{login}' (offline or blocked)")
        })?;
        let sig = pat.get("signature").and_then(|s| s.as_str()).ok_or_else(|| {
            format!("Twitch returned no playback token for '{login}' (offline or blocked)")
        })?;
        let tok = pat.get("value").and_then(|s| s.as_str()).ok_or_else(|| {
            format!("Twitch returned no playback token for '{login}' (offline or blocked)")
        })?;
        Ok((sig.to_string(), tok.to_string()))
    }

    /// Resolve a specific quality from the cached master (cache-first).
    pub async fn get_quality_url(&self, provider_id: &str, quality: &str) -> Result<Value, String> {
        let login = Self::normalize_login(provider_id);
        if login.is_empty() {
            return Err("Missing providerId for Twitch stream lookup".to_string());
        }
        let variants = self.master_variants(&login).await?;
        self.select_and_store(&login, variants, quality).await
    }

    /// Parsed master variants for a channel: cache-first, fresh resolve + store
    /// on miss (signed Usher URLs expire, hence the short TTL).
    async fn master_variants(&self, login: &str) -> Result<Vec<hls::Variant>, String> {
        if let Some(cached) = self.masters.get(login) {
            if let Some(arr) = cached.as_array() {
                let variants: Vec<hls::Variant> = arr
                    .iter()
                    .filter_map(|v| {
                        Some(hls::Variant {
                            name: v.get("name")?.as_str()?.to_string(),
                            group_id: v.get("group")?.as_str().unwrap_or("").to_string(),
                            width: v.get("width").and_then(|x| x.as_u64()).map(|x| x as u32),
                            height: v.get("height").and_then(|x| x.as_u64()).map(|x| x as u32),
                            fps: v.get("fps").and_then(|x| x.as_f64()),
                            codecs: v.get("codecs").and_then(|x| x.as_str()).map(str::to_string),
                            bandwidth: v.get("bandwidth").and_then(|x| x.as_u64()),
                            url: v.get("url")?.as_str()?.to_string(),
                        })
                    })
                    .collect();
                if !variants.is_empty() {
                    return Ok(variants);
                }
            }
        }
        let (sig, tok) = self.playback_token(login).await?;
        let usher = format!(
            "https://usher.ttvnw.net/api/channel/hls/{login}.m3u8?platform=web&player_type=embed&allow_source=true&allow_audio_only=true&playlist_include_framerate=true&supported_codecs=av1,h264,h265&fast_bread=true&include_unavailable=true&sig={sig}&token={}",
            url::form_urlencoded::byte_serialize(tok.as_bytes()).collect::<String>()
        );
        let res = self
            .ctx
            .http
            .get(&usher)
            .header("Referer", REFERER)
            .header("Origin", "https://player.twitch.tv")
            .header("User-Agent", UA)
            .send()
            .await
            .map_err(|e| format!("Twitch playlist fetch failed: {e}"))?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(format!("{login} isn't live right now"));
        }
        if !res.status().is_success() {
            return Err(format!("Twitch playlist fetch failed ({})", res.status()));
        }
        let master = res.text().await.map_err(|e| format!("Twitch read failed: {e}"))?;
        let variants = hls::parse_master(&master, &usher);
        if variants.is_empty() {
            return Err(format!("Twitch master playlist for '{login}' had no variants"));
        }
        let serial: Vec<Value> = variants
            .iter()
            .map(|v| {
                json!({
                    "name": v.name, "group": v.group_id, "width": v.width,
                    "height": v.height, "fps": v.fps, "codecs": v.codecs,
                    "bandwidth": v.bandwidth, "url": v.url,
                })
            })
            .collect();
        self.masters.set(login, json!(serial));
        Ok(variants)
    }

    pub async fn get_stream_url(
        &self,
        provider_id: Option<&str>,
        episode: &str,
        _mode: StreamMode,
    ) -> Result<Value, String> {
        let login = Self::normalize_login(provider_id.unwrap_or_default());
        if login.is_empty() {
            return Err("Missing providerId for Twitch stream lookup".to_string());
        }
        // Live channels expose a single "live" entry; tolerate numeric tokens
        // from anime-shaped callers ("1").
        let ep = episode.trim();
        if !ep.is_empty() && ep != "live" && ep != "1" {
            return Err(format!("Twitch live only supports episode 'live', got '{episode}'"));
        }
        let variants = self.master_variants(&login).await?;
        self.select_and_store(&login, variants, "best").await
    }

    async fn select_and_store(
        &self,
        _login: &str,
        variants: Vec<hls::Variant>,
        quality: &str,
    ) -> Result<Value, String> {
        let (idx, label) = hls::select_variant(&variants, quality)
            .ok_or_else(|| "Twitch: no usable quality for this stream".to_string())?;
        Ok(json!({
            "url": variants[idx].url,
            "referer": REFERER,
            "qualities": hls::qualities_json(&variants),
            "selectedQuality": label,
        }))
    }

    /// Quality menu without resolving playback (parsed master cache or fresh).
    pub async fn qualities(&self, provider_id: &str) -> Result<Value, String> {
        let login = Self::normalize_login(provider_id);
        if let Some(cached) = self.masters.get(&login) {
            if let Some(arr) = cached.as_array() {
                if !arr.is_empty() {
                    return Ok(json!({"qualities": arr}));
                }
            }
        }
        let v = self.get_stream_url(Some(&login), "live", StreamMode::Sub).await?;
        Ok(json!({ "qualities": v.get("qualities").cloned().unwrap_or(json!([])) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::framework::tests::live_or_skip;

    #[tokio::test]
    async fn parse_rows_shape() {
        let p = TwitchProvider::new(ProviderContext::new());
        let user = json!({
            "login": "shroud",
            "displayName": "Shroud",
            "profileImageURL": "https://example/avatar.png",
            "stream": {
                "id": "1", "title": "Ranked",
                "viewersCount": 12345, "createdAt": "2026-01-01T00:00:00Z",
                "game": { "id": "1", "displayName": "Shooter" },
                "previewImageURL": "https://example/preview.jpg"
            },
            "broadcastSettings": {}
        });
        let row = p.row_from_user(&user).expect("row");
        assert_eq!(row["views"], json!(12345));
        assert_eq!(row["isLive"], json!(true));
        assert_eq!(row["providerId"], json!("shroud"));
    }

    #[tokio::test]
    async fn live_channel_lookup() {
        let p = TwitchProvider::new(ProviderContext::new());
        // Shape check via direct GQL; skip on transport blocks.
        let res = p.fetch_user("shroud").await.map(|u| json!(u));
        let _ = live_or_skip(res.map_err(|e| e), "twitch lookup");
    }
}
