//! YouTube provider — anonymous InnerTube (no API key registration).
//!
//! Live streams + regular videos via `youtubei/v1/player` as rotation-tolerant
//! clients (ANDROID → WEB_EMBEDDED). Content only: no chat, no subscriptions.

use serde_json::{json, Value};

use super::framework::{as_str, ProviderContext, StreamMode};
use super::hls;
use super::TtlCache;

const INNERTUBE: &str = "https://www.youtube.com/youtubei/v1";
const INNERTUBE_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const REFERER: &str = "https://www.youtube.com/";
const SEARCH_LIMIT: usize = 12;

struct PlayerClient {
    name: &'static str,
    client_name: &'static str,
    client_version: &'static str,
    user_agent: &'static str,
}

const CLIENTS: &[PlayerClient] = &[
    PlayerClient {
        name: "android",
        client_name: "ANDROID",
        client_version: "21.08.266",
        user_agent: "com.google.android.youtube/21.08.266 (Linux; U; Android 14) gzip",
    },
    PlayerClient {
        name: "web_embedded",
        client_name: "WEB_EMBEDDED_PLAYER",
        client_version: "1.20240101.00.00",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36",
    },
];

pub struct YouTubeProvider {
    ctx: ProviderContext,
    masters: TtlCache,
}

impl YouTubeProvider {
    pub fn new(ctx: ProviderContext) -> Self {
        Self { ctx, masters: TtlCache::new(110) }
    }

    fn is_video_id(s: &str) -> bool {
        s.len() == 11
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    fn live_page_url(id: &str) -> String {
        let id = id.trim();
        if id.starts_with('@') {
            format!("https://www.youtube.com/{id}/live")
        } else if id.len() == 24 && id.to_ascii_lowercase().starts_with("uc") {
            format!("https://www.youtube.com/channel/{id}/live")
        } else if Self::is_video_id(id) {
            format!("https://www.youtube.com/watch?v={id}")
        } else {
            format!("https://www.youtube.com/@{id}/live")
        }
    }

    async fn fetch_html(&self, url: &str) -> Result<String, String> {
        let res = self
            .ctx
            .http
            .get(url)
            .header("Cookie", "SOCS=CAI; PREF=hl=en&tz=UTC")
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| format!("YouTube request failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("YouTube request failed ({}): {url}", res.status()));
        }
        res.text().await.map_err(|e| format!("YouTube read failed: {e}"))
    }

    fn extract_json_block<'a>(html: &'a str, marker: &str) -> Option<Value> {
        let start = html.find(marker)?;
        let after = &html[start + marker.len()..];
        let after = after.trim_start().trim_start_matches('=').trim_start();
        let json_start = after.find('{')?;
        let slice = &after[json_start..];
        // Brace-match to the closing object (tolerant, no full JS parse).
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        for (i, c) in slice.char_indices() {
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) = serde_json::from_str::<Value>(&slice[..i + 1]) {
                            return Some(v);
                        }
                        return None;
                    }
                }
                _ => {}
            }
        }
        None
    }

    async fn live_video_id(&self, ident: &str) -> Result<String, String> {
        let ident = ident.trim();
        if Self::is_video_id(ident) {
            return Ok(ident.to_string());
        }
        let html = self.fetch_html(&Self::live_page_url(ident)).await?;
        if let Some(player) = Self::extract_json_block(&html, "ytInitialPlayerResponse") {
            let upcoming = player
                .pointer("/videoDetails/isUpcoming")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let status = player
                .pointer("/playabilityStatus/status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if status == "LIVE_STREAM_OFFLINE" || upcoming {
                let name = player
                    .pointer("/videoDetails/author")
                    .and_then(|v| v.as_str())
                    .unwrap_or(ident);
                return Err(format!("{name} isn't live right now"));
            }
            if let Some(id) = player
                .pointer("/videoDetails/videoId")
                .and_then(|v| v.as_str())
            {
                if !id.is_empty() {
                    return Ok(id.to_string());
                }
            }
        }
        Err(format!("{ident} isn't live right now"))
    }

    async fn player(&self, video_id: &str, client: &PlayerClient) -> Result<Value, String> {
        let body = json!({
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
            "context": { "client": {
                "clientName": client.client_name,
                "clientVersion": client.client_version,
                "hl": "en", "gl": "US",
            }},
        });
        let res = self
            .ctx
            .http
            .post(format!("{INNERTUBE}/player?key={INNERTUBE_KEY}"))
            .header("User-Agent", client.user_agent)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("YouTube player request failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("YouTube player returned {}", res.status()));
        }
        res.json::<Value>().await.map_err(|e| format!("YouTube read failed: {e}"))
    }

    /// Height parsed from a "720p" label for closest-match selection.
    fn height_of_label(label: &str) -> Option<u32> {
        label
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    }

    /// All progressive MP4 formats as `(label, url)`, ascending quality.
    /// Labels come from InnerTube `qualityLabel` ("360p") with a height
    /// fallback so the quality menu has one entry per rendition.
    /// Every URL carries `ratebypass=yes`: without it YouTube throttles
    /// progressive delivery to ~realtime, which stalls seeks past the
    /// playhead (the player can only ever play what has trickled in).
    fn progressive_formats(player: &Value) -> Vec<(String, String)> {
        let formats = player
            .pointer("/streamingData/formats")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for f in &formats {
            let Some(url) = f.get("url").and_then(|u| u.as_str()).filter(|u| !u.is_empty()) else {
                continue;
            };
            let url = with_ratebypass(url);
            let label = f
                .get("qualityLabel")
                .and_then(|q| q.as_str())
                .filter(|q| !q.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    f.get("height")
                        .and_then(|h| h.as_u64())
                        .map(|h| format!("{h}p"))
                })
                .unwrap_or_else(|| "best".to_string());
            if out.iter().any(|(l, _)| l == &label) {
                continue;
            }
            out.push((label, url.to_string()));
        }
        out
    }

    fn playability_error(player: &Value) -> Option<String> {        let status = player.pointer("/playabilityStatus/status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "OK" {
            return None;
        }
        let reason = player
            .pointer("/playabilityStatus/reason")
            .and_then(|v| v.as_str())
            .unwrap_or(status);
        Some(reason.to_string())
    }

    /// Resolve a specific quality (cache-first, no extra network on repeat).
    pub async fn get_quality_url(&self, video_ref: &str, quality: &str) -> Result<Value, String> {
        let video_id = if Self::is_video_id(video_ref.trim()) {
            video_ref.trim().to_string()
        } else {
            self.live_video_id(video_ref).await?
        };
        let (url, variants, duration, label) = self.resolve_full(&video_id, quality).await?;
        Ok(json!({
            "url": url,
            "referer": REFERER,
            "qualities": hls::qualities_json(&variants),
            "selectedQuality": label,
            "durationMs": duration,
        }))
    }

    /// All playable variants for a video: HLS ladder for live, progressive
    /// MP4 formats for VODs. Cache-first. Returns variants + total duration
    /// (from `videoDetails.lengthSeconds`) so VOD players can show a real
    /// seekbar even when the media bytes alone report no duration.
    async fn variants_for(&self, video_id: &str) -> Result<(Vec<hls::Variant>, Option<u64>), String> {
        if let Some(cached) = self.masters.get(video_id) {
            let (variants, duration) = deser_cached(&cached);
            if !variants.is_empty() {
                // Progressive (single-"best") entries are revalidated below
                // only when stale; HLS masters are re-fetched on miss.
                return Ok((variants, duration));
            }
        }
        let mut last_err = String::from("no client returned a manifest");
        for client in CLIENTS {
            let player = match self.player(video_id, client).await {
                Ok(p) => p,
                Err(e) => {
                    last_err = e;
                    continue;
                }
            };
            if let Some(reason) = Self::playability_error(&player) {
                last_err = reason;
                continue;
            }
            // Live: HLS manifest. VODs have no HLS — fall back to the
            // progressive MP4 formats (direct URLs, one menu entry each).
            if player.pointer("/streamingData/hlsManifestUrl").and_then(|v| v.as_str()).is_none() {
                let formats = Self::progressive_formats(&player);
                if formats.is_empty() {
                    last_err = "no playable format".to_string();
                    continue;
                }
                let variants: Vec<hls::Variant> = formats
                    .iter()
                    .map(|(label, url)| hls::Variant {
                        name: label.clone(),
                        group_id: String::new(),
                        width: None,
                        height: Self::height_of_label(label),
                        fps: None,
                        codecs: None,
                        bandwidth: None,
                        url: url.clone(),
                    })
                    .collect();
                self.masters.set(video_id, ser_cached(&variants, duration_ms(&player)));
                return Ok((variants, duration_ms(&player)));
            }
            let Some(manifest) = player
                .pointer("/streamingData/hlsManifestUrl")
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let manifest = manifest.to_string();
            let res = self.ctx.http.get(&manifest).send().await.map_err(|e| format!("YouTube manifest fetch failed: {e}"))?;
            if !res.status().is_success() {
                last_err = format!("YouTube manifest returned {}", res.status());
                continue;
            }
            let text = res.text().await.map_err(|e| format!("YouTube read failed: {e}"))?;
            let base = manifest.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
            let variants = hls::parse_master(&text, base);
            if variants.is_empty() {
                last_err = "master playlist had no variants".to_string();
                continue;
            }
            self.masters.set(video_id, ser_cached(&variants, duration_ms(&player)));
            return Ok((variants, duration_ms(&player)));
        }
        Err(format!("couldn't resolve a YouTube stream for '{video_id}': {last_err}"))
    }

    fn map_video_renderer(&self, vr: &Value) -> Option<Value> {
        let video_id = as_str(vr.get("videoId"))?;
        let title_runs = vr.pointer("/title/runs").and_then(|v| v.as_array());
        let title = title_runs
            .map(|runs| runs.iter().filter_map(|r| r.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
            .filter(|s| !s.is_empty())
            .or_else(|| as_str(vr.pointer("/title/simpleText")))
            .unwrap_or_else(|| video_id.clone());
        let channel = as_str(vr.pointer("/longBylineText/runs/0/text"))
            .or_else(|| as_str(vr.pointer("/ownerText/runs/0/text")))
            .unwrap_or_default();
        let thumb = vr
            .pointer("/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|ts| ts.last())
            .and_then(|t| as_str(t.get("url")))
            .unwrap_or_default();
        let views_text = as_str(vr.get("viewCountText").and_then(|v| v.get("simpleText")))
            .or_else(|| {
                vr.get("viewCountText")
                    .and_then(|v| v.get("runs"))
                    .and_then(|v| v.as_array())
                    .map(|runs| runs.iter().filter_map(|r| r.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
            })
            .unwrap_or_default();
        let views = parse_views(&views_text);
        let is_live = vr
            .pointer("/badges")
            .and_then(|v| v.as_array())
            .map(|b| {
                b.iter().any(|x| {
                    x.pointer("/metadataBadgeRenderer/label")
                        .and_then(|l| l.as_str())
                        .map(|l| l.to_ascii_lowercase().contains("live"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or_else(|| views_text.to_ascii_lowercase().contains("watching"));
        Some(json!({
            "id": format!("youtube:{video_id}"),
            "providerId": video_id,
            "title": { "english": title, "romanji": channel },
            "thumbnail": if thumb.is_empty() { Value::Null } else { json!(thumb) },
            "availableEpisodes": { "sub": 1 },
            "viewerCount": views,
            "views": views,
            "isLive": is_live,
            "category": channel,
            "channelName": channel,
            "type": if is_live { "LIVE" } else { "VIDEO" },
            "status": if is_live { "LIVE" } else { "VOD" },
        }))
    }

    fn collect_renderers<'a>(&self, node: &'a Value, out: &mut Vec<Value>) {
        if out.len() >= SEARCH_LIMIT {
            return;
        }
        match node {
            Value::Object(map) => {
                if let Some(vr) = map.get("videoRenderer") {
                    if let Some(row) = self.map_video_renderer(vr) {
                        out.push(row);
                    }
                    return;
                }
                for v in map.values() {
                    if out.len() >= SEARCH_LIMIT {
                        return;
                    }
                    self.collect_renderers(v, out);
                }
            }
            Value::Array(arr) => {
                for v in arr {
                    if out.len() >= SEARCH_LIMIT {
                        return;
                    }
                    self.collect_renderers(v, out);
                }
            }
            _ => {}
        }
    }

    async fn innertube_search(&self, query: &str) -> Result<Value, String> {
        let body = json!({
            "query": query,
            "context": { "client": {
                "clientName": "WEB", "clientVersion": "2.20240101.00.00",
                "hl": "en", "gl": "US",
            }},
        });
        let res = self
            .ctx
            .http
            .post(format!("{INNERTUBE}/search?key={INNERTUBE_KEY}"))
            .header("User-Agent", CLIENTS[1].user_agent)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("YouTube search failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("YouTube search failed ({})", res.status()));
        }
        res.json::<Value>().await.map_err(|e| format!("YouTube read failed: {e}"))
    }

    pub async fn search(&self, query: &str) -> Result<Value, String> {
        let q = query.trim();
        if q.is_empty() {
            return self.get_recent_uploads(1, SEARCH_LIMIT, StreamMode::Sub).await;
        }
        if Self::is_video_id(q) {
            return Ok(json!([json!({
                "id": format!("youtube:{q}"),
                "providerId": q,
                "title": { "english": q },
                "thumbnail": format!("https://i.ytimg.com/vi/{q}/hqdefault.jpg"),
                "availableEpisodes": { "sub": 1 },
                "views": 0, "viewerCount": 0, "isLive": false,
                "type": "VIDEO",
            })]));
        }
        let v = self.innertube_search(q).await?;
        let mut out = Vec::new();
        self.collect_renderers(&v, &mut out);
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
        // Live directory approximation: search for live content.
        let v = self.innertube_search("live").await?;
        let mut out = Vec::new();
        self.collect_renderers(&v, &mut out);
        out.truncate(limit.clamp(1, SEARCH_LIMIT));
        // Episode-shaped rows for the home carousel.
        let eps: Vec<Value> = out
            .into_iter()
            .map(|r| {
                json!({
                    "id": r["id"],
                    "providerId": r["providerId"],
                    "title": r["title"],
                    "thumbnail": r["thumbnail"],
                    "index": 1,
                    "mode": "sub",
                    "viewerCount": r["viewerCount"],
                    "views": r["views"],
                    "isLive": r["isLive"],
                    "category": r["category"],
                })
            })
            .collect();
        Ok(json!(eps))
    }

    pub async fn get_episodes_list(&self, provider_id: &str) -> Result<Value, String> {
        let id = provider_id.trim();
        if id.is_empty() {
            return Err("Missing providerId for YouTube lookup".to_string());
        }
        if Self::is_video_id(id) {
            return Ok(json!([id]));
        }
        // Channel → current live broadcast.
        let video = self.live_video_id(id).await?;
        Ok(json!([video]))
    }

    pub async fn get_show_details(&self, provider_id: &str) -> Result<Value, String> {
        let id = provider_id.trim();
        if Self::is_video_id(id) {
            return Ok(json!({
                "id": format!("youtube:{id}"),
                "providerId": id,
                "name": id,
                "thumbnail": format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
                "type": "VIDEO",
                "description": Value::Null,
                "isLive": false,
                "views": 0,
            }));
        }
        match self.live_video_id(id).await {
            Ok(video) => Ok(json!({
                "id": format!("youtube:{video}"),
                "providerId": video,
                "name": id,
                "thumbnail": format!("https://i.ytimg.com/vi/{video}/hqdefault.jpg"),
                "type": "LIVE",
                "description": Value::Null,
                "isLive": true,
            })),
            Err(e) => Ok(json!({
                "id": format!("youtube:{id}"),
                "providerId": id,
                "name": id,
                "thumbnail": Value::Null,
                "type": "CHANNEL",
                "description": e,
                "isLive": false,
            })),
        }
    }

    pub async fn get_stream_url(
        &self,
        provider_id: Option<&str>,
        episode: &str,
        _mode: StreamMode,
    ) -> Result<Value, String> {
        let ep = episode.trim();
        let pid = provider_id.unwrap_or_default().trim();
        let video_id = if Self::is_video_id(ep) {
            ep.to_string()
        } else if Self::is_video_id(pid) {
            pid.to_string()
        } else if !pid.is_empty() {
            self.live_video_id(pid).await?
        } else if !ep.is_empty() && ep != "live" && ep != "1" {
            self.live_video_id(ep).await?
        } else {
            return Err("Missing video id for YouTube stream lookup".to_string());
        };
        let quality = "best";
        let (url, variants, duration_ms, label) = self.resolve_full(&video_id, quality).await?;
        Ok(json!({
            "url": url,
            "referer": REFERER,
            "qualities": hls::qualities_json(&variants),
            "selectedQuality": label,
            "durationMs": duration_ms,
        }))
    }

    async fn resolve_full(
        &self,
        video_id: &str,
        quality: &str,
    ) -> Result<(String, Vec<hls::Variant>, Option<u64>, String), String> {
        let (variants, duration) = self.variants_for(video_id).await?;
        let (idx, label) = hls::select_variant(&variants, quality)
            .ok_or_else(|| "YouTube: no usable quality".to_string())?;
        Ok((variants[idx].url.clone(), variants, duration, label))
    }
}

fn deser_cached(cached: &Value) -> (Vec<hls::Variant>, Option<u64>) {
    let arr = cached.get("variants").and_then(|v| v.as_array());
    let duration = cached.get("durationMs").and_then(|v| v.as_u64());
    let variants = arr
        .map(|a| deser_variants(a))
        .unwrap_or_default();
    (variants, duration)
}

/// Append `ratebypass=yes` so YouTube serves progressive MP4 at full speed
/// instead of throttling to ~realtime (which breaks seeking past the playhead).
fn with_ratebypass(url: &str) -> String {
    if url.contains("ratebypass=") {
        return url.to_string();
    }
    if url.contains('?') {
        format!("{url}&ratebypass=yes")
    } else {
        format!("{url}?ratebypass=yes")
    }
}

/// Total video length in ms from `videoDetails.lengthSeconds`.
fn duration_ms(player: &Value) -> Option<u64> {
    let v = player.pointer("/videoDetails/lengthSeconds")?;
    let secs: u64 = match v {
        Value::String(s) => s.parse().ok()?,
        Value::Number(n) => n.as_u64()?,
        _ => return None,
    };
    if secs == 0 {
        None
    } else {
        Some(secs.saturating_mul(1000))
    }
}

fn deser_variants(arr: &[Value]) -> Vec<hls::Variant> {
    arr.iter()
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
        .collect()
}

fn ser_cached(variants: &[hls::Variant], duration_ms: Option<u64>) -> Value {
    json!({
        "variants": variants
            .iter()
            .map(|v| json!({
                "name": v.name, "group": v.group_id, "width": v.width,
                "height": v.height, "fps": v.fps, "codecs": v.codecs,
                "bandwidth": v.bandwidth, "url": v.url,
            }))
            .collect::<Vec<_>>(),
        "durationMs": duration_ms,
    })
}

fn parse_views(s: &str) -> u64 {
    let s = s.to_lowercase();
    let num: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    let n: f64 = num.replace(',', "").parse().unwrap_or(0.0);
    let mult = if s.contains('m') { 1_000_000.0 } else if s.contains('k') { 1_000.0 } else { 1.0 };
    (n * mult) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_id_shape() {
        assert!(YouTubeProvider::is_video_id("dQw4w9WgXcQ"));
        assert!(!YouTubeProvider::is_video_id("shroud"));
    }

    #[test]
    fn views_parse() {
        assert_eq!(parse_views("12.3K views"), 12300);
        assert_eq!(parse_views("1.2M views"), 1200000);
    }

    #[test]
    fn ratebypass_appended_once() {
        assert_eq!(
            super::with_ratebypass("https://r1.googlevideo.com/videoplayback?expire=1&n=abc"),
            "https://r1.googlevideo.com/videoplayback?expire=1&n=abc&ratebypass=yes"
        );
        assert_eq!(
            super::with_ratebypass("https://x/y?ratebypass=yes"),
            "https://x/y?ratebypass=yes"
        );
    }

    #[test]
    fn renderer_maps_views() {
        let p = YouTubeProvider::new(ProviderContext::new());
        let vr = json!({
            "videoId": "dQw4w9WgXcQ",
            "title": { "runs": [{ "text": "Hello" }] },
            "longBylineText": { "runs": [{ "text": "SomeChannel" }] },
            "thumbnail": { "thumbnails": [{ "url": "https://i.ytimg.com/vi/x/hqdefault.jpg" }] },
            "viewCountText": { "simpleText": "1.2M views" },
        });
        let row = p.map_video_renderer(&vr).expect("row");
        assert_eq!(row["views"], json!(1200000));
    }
}
