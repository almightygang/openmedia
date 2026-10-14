//! Pure HLS / subtitle helpers — direct port of
//! `shared/utils/hls-url.ts`, `shared/utils/hls-master.ts`,
//! `main/utils/ass-to-webvtt.ts` and the manifest/playlist builders
//! from `main/stream-proxy.ts`.

use sha1::{Digest, Sha1};

// ---------------------------------------------------------------------------
// hls-url
// ---------------------------------------------------------------------------

/// True when `url` points at an HLS master or media playlist (not a segment).
pub fn is_hls_playlist_url(url: &str) -> bool {
    if let Ok(parsed) = url::Url::parse(url) {
        let path = parsed.path().to_lowercase();
        return path == "m3u8"
            || path.ends_with("/m3u8")
            || path.ends_with(".m3u8");
    }
    let lower = url.to_lowercase();
    let base = lower.split(['?', '#']).next().unwrap_or(&lower);
    base == "m3u8" || base.ends_with("/m3u8") || base.ends_with(".m3u8")
}

// ---------------------------------------------------------------------------
// hls-master
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HlsVideoVariant {
    /// Relative or absolute URI from the master line after STREAM-INF.
    pub uri: String,
    pub bandwidth: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// UI label, e.g. "1080p".
    pub label: String,
}

fn parse_stream_inf_attrs(line: &str) -> (Option<u64>, Option<u32>, Option<u32>) {
    let bandwidth = line
        .split(',')
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("BANDWIDTH=")
                .or_else(|| strip_case_prefix(part, "bandwidth="))
                .and_then(|v| v.parse::<u64>().ok())
        });
    let (width, height): (Option<u32>, Option<u32>) = line
        .split(',')
        .find_map(|part| {
            let part = part.trim();
            let v = part
                .strip_prefix("RESOLUTION=")
                .or_else(|| strip_case_prefix(part, "resolution="))?;
            let (w, h) = v.split_once(['x', 'X'])?;
            Some((
                w.parse::<u32>().ok()?,
                h.trim().parse::<u32>().ok()?,
            ))
        })
        .map(|(w, h)| (Some(w), Some(h)))
        .unwrap_or((None, None));
    (bandwidth, width, height)
}

fn strip_case_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn variant_label(height: Option<u32>, bandwidth: Option<u64>) -> String {
    if let Some(h) = height {
        if h > 0 {
            return format!("{h}p");
        }
    }
    if let Some(b) = bandwidth {
        if b > 0 {
            return format!("{}kbps", (b as f64 / 1000.0).round() as u64);
        }
    }
    "Auto".to_string()
}

/// Parse video renditions from an HLS master playlist body.
pub fn parse_hls_video_variants(master: &str) -> Vec<HlsVideoVariant> {
    let lines: Vec<&str> = master.split_terminator('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line.starts_with("#EXT-X-STREAM-INF") {
            let next = lines.get(i + 1).map(|l| l.trim_end_matches('\r').trim().to_string());
            if let Some(next) = next {
                if !next.is_empty() && !next.starts_with('#') {
                    let (bandwidth, width, height) = parse_stream_inf_attrs(line);
                    out.push(HlsVideoVariant {
                        uri: next,
                        bandwidth,
                        width,
                        height,
                        label: variant_label(height, bandwidth),
                    });
                }
            }
        }
        i += 1;
    }
    out
}

/// Sort tallest → shortest (then bandwidth).
pub fn sort_hls_variants_descending(mut variants: Vec<HlsVideoVariant>) -> Vec<HlsVideoVariant> {
    variants.sort_by(|a, b| {
        (b.height.unwrap_or(0))
            .cmp(&a.height.unwrap_or(0))
            .then((b.bandwidth.unwrap_or(0)).cmp(&a.bandwidth.unwrap_or(0)))
    });
    variants
}

/// Prefer a mid/high quality that is less CDN-hostile than max 1080p.
pub fn pick_preferred_hls_variant(
    variants: &[HlsVideoVariant],
    preferred_height: u32,
) -> Option<HlsVideoVariant> {
    if variants.is_empty() {
        return None;
    }
    let sorted = sort_hls_variants_descending(variants.to_vec());
    if let Some(exact) = sorted.iter().find(|v| v.height == Some(preferred_height)) {
        return Some(exact.clone());
    }
    // Closest at-or-below preferred (sorted desc → first match is the tallest ≤ preferred).
    if let Some(below) = sorted
        .iter()
        .find(|v| matches!(v.height, Some(h) if h > 0 && h <= preferred_height))
    {
        return Some(below.clone());
    }
    // Else closest above (last element is shortest), else tallest.
    sorted.last().or(sorted.first()).cloned()
}

fn urls_loosely_equal(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let path_of = |s: &str| -> Option<String> {
        if s.contains("://") {
            url::Url::parse(s).ok().map(|u| u.path().to_string())
        } else {
            Some(s.strip_prefix("./").unwrap_or(s).to_string())
        }
    };
    match (path_of(a), path_of(b)) {
        (Some(pa), Some(pb)) => {
            pa == pb
                || pa.ends_with(&format!("/{pb}"))
                || pb.ends_with(&format!("/{pa}"))
                || pa.ends_with(&pb)
                || pb.ends_with(&pa)
        }
        _ => false,
    }
}

/// Keep AUDIO/SUBTITLE media tags and a single video STREAM-INF (+ its URI line).
pub fn filter_hls_master_to_variant(
    master: &str,
    variant_uri: Option<&str>,
    preferred_height: u32,
) -> String {
    if !master.contains("#EXT-X-STREAM-INF") {
        return master.to_string();
    }
    let variants = parse_hls_video_variants(master);
    if variants.len() <= 1 {
        return master.to_string();
    }
    let wanted = variant_uri.map(str::trim).filter(|s| !s.is_empty());
    let matched = wanted
        .and_then(|w| {
            variants
                .iter()
                .find(|v| v.uri == w || urls_loosely_equal(&v.uri, w))
                .cloned()
        })
        .or_else(|| pick_preferred_hls_variant(&variants, preferred_height));
    let matched = match matched {
        Some(m) => m,
        None => return master.to_string(),
    };

    let lines: Vec<&str> = master.split_terminator('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut kept = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line.starts_with("#EXT-X-STREAM-INF") {
            let next = lines.get(i + 1).map(|l| l.trim_end_matches('\r').trim());
            if let Some(next) = next {
                if next == matched.uri || urls_loosely_equal(next, &matched.uri) {
                    out.push(line.to_string());
                    if !next.is_empty() {
                        out.push(next.to_string());
                    }
                    kept = true;
                }
            }
            i += 2;
            continue;
        }
        out.push(line.to_string());
        i += 1;
    }
    if kept {
        out.join("\n")
    } else {
        master.to_string()
    }
}

// ---------------------------------------------------------------------------
// manifest rewrite (from stream-proxy.ts)
// ---------------------------------------------------------------------------

pub fn to_absolute_url(url: &str, base_url: &str) -> String {
    match url::Url::parse(base_url).and_then(|base| base.join(url)) {
        Ok(abs) => abs.to_string(),
        Err(_) => url.to_string(),
    }
}

/// Rewrite every segment/playlist/key URI in `manifest` to an opaque local
/// proxy URL. `register` maps an absolute upstream URL → opaque id.
pub fn rewrite_hls_manifest<F>(
    manifest: &str,
    manifest_url: &str,
    proxy_playlist_path: &str,
    proxy_segment_path: &str,
    proxy_base_url: &str,
    mut register: F,
) -> String
where
    F: FnMut(&str) -> String,
{
    let to_proxy_url = |raw_uri: &str, register: &mut F| -> String {
        let trimmed = raw_uri.trim();
        if trimmed.is_empty() || trimmed.starts_with("data:") || trimmed.starts_with("blob:") {
            return raw_uri.to_string();
        }
        let absolute = to_absolute_url(trimmed, manifest_url);
        let proxy_path = if is_hls_playlist_url(&absolute) {
            proxy_playlist_path
        } else {
            proxy_segment_path
        };
        let id = register(&absolute);
        format!("{proxy_base_url}{proxy_path}?u={id}")
    };

    manifest
        .split('\n')
        .map(|line| {
            // Preserve original line endings handling: split('\n') keeps \r at EOL;
            // trim it for matching but re-append to keep output stable.
            let (body, cr) = match line.strip_suffix('\r') {
                Some(b) => (b, "\r"),
                None => (line, ""),
            };
            if body.is_empty() {
                return line.to_string();
            }
            if body.starts_with('#') {
                if body.contains("URI=\"") {
                    let mut out = body.to_string();
                    // Replace each URI="..." occurrence.
                    let mut result = String::with_capacity(out.len() + 64);
                    let mut rest = out.as_str();
                    // Manual scan to avoid regex dependency.
                    while let Some(start) = rest.find("URI=\"") {
                        let uri_start = start + 5;
                        if let Some(end_rel) = rest[uri_start..].find('"') {
                            let uri = &rest[uri_start..uri_start + end_rel];
                            let replaced = to_proxy_url(uri, &mut register);
                            result.push_str(&rest[..uri_start]);
                            result.push_str(&replaced);
                            rest = &rest[uri_start + end_rel..];
                        } else {
                            break;
                        }
                    }
                    result.push_str(rest);
                    out = result;
                    if cr == "\r" {
                        out.push('\r');
                    }
                    return out;
                }
                return line.to_string();
            }
            let mut replaced = to_proxy_url(body, &mut register);
            if cr == "\r" {
                replaced.push('\r');
            }
            replaced
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// transcode helpers
// ---------------------------------------------------------------------------

/// sha1 hex of the transcode session material (target URL + optional variant).
pub fn transcode_cache_key(target_url: &str, variant: Option<&str>) -> String {
    let material = match variant.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => format!("{target_url}\nvariant={v}"),
        None => target_url.to_string(),
    };
    let mut h = Sha1::new();
    h.update(material.as_bytes());
    format!("{:x}", h.finalize())
}

/// Opaque upstream-target id: letter-prefixed sha1 so Windows ffmpeg never
/// mis-parses it as scientific notation.
pub fn upstream_target_id(url: &str, referer: Option<&str>) -> String {
    let mut h = Sha1::new();
    h.update(url.as_bytes());
    h.update(b"\0");
    h.update(referer.unwrap_or("").as_bytes());
    format!("t{:x}", h.finalize()).chars().take(21).collect()
}

/// Synthesize the VOD playlist served at /transcode/playlist.m3u8.
pub fn build_vod_playlist(
    segment_count: u32,
    segment_duration_secs: u32,
    target_url: &str,
    referer: Option<&str>,
    variant: Option<&str>,
    proxy_base_url: &str,
) -> String {
    let mut query = format!(
        "url={}&referer={}",
        url_encode(target_url),
        url_encode(referer.unwrap_or(""))
    );
    if let Some(v) = variant.map(str::trim).filter(|s| !s.is_empty()) {
        query.push_str(&format!("&variant={}", url_encode(v)));
    }
    let target_duration = segment_duration_secs.max(1);
    let mut lines = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:3".to_string(),
        "#EXT-X-PLAYLIST-TYPE:VOD".to_string(),
        format!("#EXT-X-TARGETDURATION:{target_duration}"),
        "#EXT-X-MEDIA-SEQUENCE:0".to_string(),
        "#EXT-X-INDEPENDENT-SEGMENTS".to_string(),
    ];
    for i in 0..segment_count {
        lines.push(format!("#EXTINF:{}.000,", segment_duration_secs));
        lines.push(format!(
            "{proxy_base_url}/transcode/segment-{i}.ts?{query}"
        ));
    }
    lines.push("#EXT-X-ENDLIST".to_string());
    lines.join("\n") + "\n"
}

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Parse a `Range: bytes=start-end` header.
pub fn parse_byte_range(header: Option<&str>) -> Option<(u64, Option<u64>)> {
    let header = header?.trim();
    let rest = header.strip_prefix("bytes=")?;
    let (start_s, end_s) = rest.split_once('-')?;
    let start: u64 = if start_s.is_empty() {
        0
    } else {
        start_s.parse().ok()?
    };
    let end: Option<u64> = if end_s.is_empty() {
        None
    } else {
        Some(end_s.parse().ok()?)
    };
    Some((start, end))
}

// ---------------------------------------------------------------------------
// ass-to-webvtt
// ---------------------------------------------------------------------------

fn subtitle_path_matches(url: &str, suffixes: &[&str]) -> bool {
    let path = match url::Url::parse(url) {
        Ok(u) => u.path().to_lowercase(),
        Err(_) => url.split(['?', '#']).next().unwrap_or(url).to_lowercase(),
    };
    suffixes.iter().any(|s| path.ends_with(s))
}

/// True when the URL looks like an ASS/SSA subtitle file.
pub fn is_ass_subtitle_url(url: &str) -> bool {
    subtitle_path_matches(url, &[".ass", ".ssa"])
}

/// True when the URL looks like a WebVTT subtitle file.
pub fn is_webvtt_subtitle_url(url: &str) -> bool {
    subtitle_path_matches(url, &[".vtt"])
}

/// Best-effort ASS/SSA → WebVTT. Keeps timing/text; drops styling.
pub fn convert_ass_to_webvtt(ass: &str) -> String {
    let mut lines = vec!["WEBVTT".to_string(), String::new()];
    let mut index = 0u32;

    for raw in ass.split(['\r', '\n']) {
        let line = raw.trim();
        if line.len() < 9 || !line[..9].eq_ignore_ascii_case("dialogue:") {
            continue;
        }
        let body = line[9..].trim_start();
        let parts: Vec<&str> = body.splitn(10, ',').collect();
        if parts.len() < 10 {
            continue;
        }
        let (Some(start), Some(end)) = (ass_time_to_vtt(parts[1]), ass_time_to_vtt(parts[2]))
        else {
            continue;
        };
        let mut text = parts[9].to_string();
        // Strip {override} tags.
        let mut stripped = String::with_capacity(text.len());
        let mut depth = 0;
        for ch in text.chars() {
            match ch {
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                _ if depth == 0 => stripped.push(ch),
                _ => {}
            }
        }
        text = stripped
            .replace("\\N", "\n")
            .replace("\\n", "\n")
            .replace("\\h", " ")
            .replace("\\H", " ")
            .trim()
            .to_string();
        // Original replaces case-insensitively; handle \N variants.
        if text.is_empty() {
            continue;
        }
        index += 1;
        lines.push(index.to_string());
        lines.push(format!("{start} --> {end}"));
        lines.push(text);
        lines.push(String::new());
    }

    lines.join("\n")
}

/// ASS `H:MM:SS.cs` → WebVTT `HH:MM:SS.mmm`.
fn ass_time_to_vtt(value: &str) -> Option<String> {
    let value = value.trim();
    let mut it = value.split(':');
    let h: u64 = it.next()?.parse().ok()?;
    let m: &str = it.next()?;
    if m.len() != 2 {
        return None;
    }
    let m: u64 = m.parse().ok()?;
    let rest: &str = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let (s, frac) = rest.split_once(['.', ':'])?;
    if s.len() != 2 {
        return None;
    }
    let s: u64 = s.parse().ok()?;
    let frac = match frac.len() {
        1 => format!("{frac}00"),
        2 => format!("{frac}0"),
        3 => frac.to_string(),
        _ => frac[..3].to_string(),
    };
    if frac.bytes().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{h:02}:{m:02}:{s:02}.{frac}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_url_detection() {
        assert!(is_hls_playlist_url("https://cdn.example.com/hls/master.m3u8"));
        assert!(!is_hls_playlist_url("https://cdn.example.com/stream?x=1.m3u8"));
        assert!(!is_hls_playlist_url("https://cdn.example.com/seg-12.ts"));
    }

    #[test]
    fn variant_parse_and_filter() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720\nhi.m3u8\n";
        let variants = parse_hls_video_variants(master);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[1].label, "720p");
        let filtered = filter_hls_master_to_variant(master, None, 720);
        assert!(filtered.contains("hi.m3u8"));
        assert!(!filtered.contains("low.m3u8"));
    }

    #[test]
    fn manifest_rewrite_uses_opaque_ids() {
        let manifest = "#EXTM3U\n#EXT-X-TARGETDURATION:6\nseg0.ts\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.key\"\n";
        let out = rewrite_hls_manifest(
            manifest,
            "https://cdn.example.com/hls/index.m3u8",
            "/stream/playlist.m3u8",
            "/stream/segment.ts",
            "http://127.0.0.1:9",
            |_| "tabc".to_string(),
        );
        assert!(out.contains("http://127.0.0.1:9/stream/segment.ts?u=tabc"));
        assert!(out.contains("URI=\"http://127.0.0.1:9/stream/segment.ts?u=tabc\""));
        assert!(!out.contains("cdn.example.com"));
    }

    #[test]
    fn ass_conversion() {
        let ass = "[Events]\nDialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,Hello{\\b1} \\Nworld";
        let vtt = convert_ass_to_webvtt(ass);
        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:03.500"));
        assert!(vtt.contains("Hello"));
    }
}
