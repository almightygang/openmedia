//! Shared HLS master-playlist parsing + variant selection.
//!
//! Clean-room implementation for live providers (Twitch/YouTube/Kick-style
//! IVS masters). Handles both legacy masters with `#EXT-X-MEDIA:TYPE=VIDEO`
//! labels and modern MEDIA-less masters where the label rides on the
//! `#EXT-X-STREAM-INF` line itself (`IVS-NAME` / `STABLE-VARIANT-ID`).

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub group_id: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codecs: Option<String>,
    pub bandwidth: Option<u64>,
    pub url: String,
}

fn extract_attr(line: &str, key: &str) -> Option<String> {
    let quoted = format!("{key}=\"");
    if let Some(pos) = line.find(&quoted) {
        let rest = &line[pos + quoted.len()..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    let needle = format!("{key}=");
    let pos = line.find(&needle)?;
    let rest = &line[pos + needle.len()..];
    let end = rest.find(',').unwrap_or(rest.len());
    let v = rest[..end].trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn parse_resolution(inf: &str) -> (Option<u32>, Option<u32>) {
    let Some(res) = extract_attr(inf, "RESOLUTION") else {
        return (None, None);
    };
    let mut parts = res.split(['x', 'X']);
    let w = parts.next().and_then(|p| p.trim().parse().ok());
    let h = parts.next().and_then(|p| p.trim().parse().ok());
    (w, h)
}

fn derive_name(height: Option<u32>, fps: Option<f64>, codecs: Option<&str>) -> Option<String> {
    match height {
        Some(h) => {
            let f = fps.unwrap_or(30.0).round() as u32;
            if f >= 50 {
                Some(format!("{h}p{f}"))
            } else {
                Some(format!("{h}p"))
            }
        }
        None => {
            let audio = codecs.is_some_and(|c| {
                c.contains("mp4a")
                    && !c.contains("avc")
                    && !c.contains("av01")
                    && !c.contains("hvc")
                    && !c.contains("hev")
            });
            audio.then(|| "audio_only".to_string())
        }
    }
}

/// Parse a master playlist into video renditions.
/// `base_url` resolves relative variant URIs.
pub fn parse_master(master: &str, base_url: &str) -> Vec<Variant> {
    let lines: Vec<&str> = master.lines().collect();
    let mut media_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for line in &lines {
        let l = line.trim();
        if l.starts_with("#EXT-X-MEDIA:") && l.contains("TYPE=VIDEO") {
            if let (Some(gid), Some(name)) =
                (extract_attr(l, "GROUP-ID"), extract_attr(l, "NAME"))
            {
                if !name.is_empty() {
                    media_names.insert(gid, name);
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let inf = lines[i].trim();
        if !inf.starts_with("#EXT-X-STREAM-INF:") {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() {
            let u = lines[j].trim();
            if u.is_empty() || u.starts_with('#') {
                j += 1;
                continue;
            }
            break;
        }
        if j >= lines.len() {
            break;
        }
        let (width, height) = parse_resolution(inf);
        let fps: Option<f64> = extract_attr(inf, "FRAME-RATE").and_then(|s| s.parse().ok());
        let codecs = extract_attr(inf, "CODECS");
        let bandwidth: Option<u64> = extract_attr(inf, "BANDWIDTH").and_then(|s| s.parse().ok());
        let video_group = extract_attr(inf, "VIDEO");
        let is_source = video_group.as_deref() == Some("chunked")
            || extract_attr(inf, "IVS-VARIANT-SOURCE").is_some_and(|s| s.eq_ignore_ascii_case("source"));

        let name = video_group
            .as_ref()
            .and_then(|g| media_names.get(g).cloned())
            .filter(|n| !n.is_empty())
            .or_else(|| extract_attr(inf, "IVS-NAME"))
            .or_else(|| extract_attr(inf, "STABLE-VARIANT-ID"))
            .or_else(|| derive_name(height, fps, codecs.as_deref()))
            .or_else(|| video_group.clone())
            .unwrap_or_default();
        if name.is_empty() {
            i = j + 1;
            continue;
        }

        let raw = lines[j].trim();
        let url = if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_string()
        } else if let Ok(base) = url::Url::parse(base_url) {
            base.join(raw).map(|u| u.to_string()).unwrap_or_else(|_| raw.to_string())
        } else {
            raw.to_string()
        };

        out.push(Variant {
            name,
            group_id: if is_source {
                "chunked".to_string()
            } else {
                video_group.clone().unwrap_or_default()
            },
            width,
            height,
            fps,
            codecs,
            bandwidth,
            url,
        });
        i = j + 1;
    }
    out
}

fn source_index(variants: &[Variant]) -> Option<usize> {
    if let Some(i) = variants.iter().position(|v| v.group_id.eq_ignore_ascii_case("chunked")) {
        return Some(i);
    }
    variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.height.is_some())
        .max_by(|(_, a), (_, b)| {
            a.height.cmp(&b.height).then(
                a.fps
                    .partial_cmp(&b.fps)
                    .unwrap_or(std::cmp::Ordering::Equal),
            ).then(a.bandwidth.cmp(&b.bandwidth))
        })
        .map(|(i, _)| i)
        .or(Some(0))
}

/// Map a requested quality to a variant index + served label.
pub fn select_variant(variants: &[Variant], requested: &str) -> Option<(usize, String)> {
    if variants.is_empty() {
        return None;
    }
    match requested.trim().to_lowercase().as_str() {
        "best" | "source" => return source_index(variants).map(|i| (i, "best".to_string())),
        "worst" => {
            let idx = variants
                .iter()
                .enumerate()
                .filter(|(_, v)| v.height.is_some())
                .min_by(|(_, a), (_, b)| {
                    a.height.cmp(&b.height).then(
                        a.fps
                            .partial_cmp(&b.fps)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            return Some((idx, "worst".to_string()));
        }
        "audio_only" | "audio-only" | "audio" => {
            if let Some(i) = variants.iter().position(|v| v.name.to_lowercase().contains("audio")) {
                return Some((i, "audio_only".to_string()));
            }
            return source_index(variants).map(|i| (i, "best".to_string()));
        }
        _ => {}
    }
    // Exact (case-insensitive) match first.
    if let Some(i) = variants.iter().position(|v| v.name.eq_ignore_ascii_case(requested.trim())) {
        let label = variants[i].name.clone();
        return Some((i, label));
    }
    // Closest by height (e.g. saved "720p" vs shipped "720p60").
    let want_h: Option<u32> = requested
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok();
    if let Some(h) = want_h {
        let mut best: Option<(usize, u32)> = None;
        for (i, v) in variants.iter().enumerate() {
            if let Some(vh) = v.height {
                let d = vh.abs_diff(h);
                if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some((i, d));
                }
            }
        }
        if let Some((i, _)) = best {
            let label = variants[i].name.clone();
            return Some((i, label));
        }
    }
    source_index(variants).map(|i| (i, variants[i].name.clone()))
}

pub fn quality_names(variants: &[Variant]) -> Vec<String> {
    let mut names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
    if !names.iter().any(|n| n.eq_ignore_ascii_case("best")) {
        names.push("best".to_string());
    }
    if !names.iter().any(|n| n.eq_ignore_ascii_case("worst")) {
        names.push("worst".to_string());
    }
    names
}

pub fn qualities_json(variants: &[Variant]) -> Value {
    json!(
        variants
            .iter()
            .map(|v| json!({
                "id": v.name,
                "label": v.name,
                "height": v.height,
                "bandwidth": v.bandwidth,
            }))
            .collect::<Vec<_>>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60\",AUTOSELECT=YES,DEFAULT=YES\n\
#EXT-X-STREAM-INF:BANDWIDTH=8000000,RESOLUTION=1920x1080,CODECS=\"avc1.4D402A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
https://cdn.example/chunked.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"480p30\",NAME=\"480p30\",AUTOSELECT=YES,DEFAULT=NO\n\
#EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=852x480,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"480p30\",FRAME-RATE=30.000\n\
https://cdn.example/480.m3u8\n";

    const MODERN: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",STABLE-VARIANT-ID=\"audio_only\",IVS-NAME=\"audio_only\",IVS-VARIANT-SOURCE=\"transcode\"\n\
https://cdn.example/audio.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=6895917,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",FRAME-RATE=60.000,STABLE-VARIANT-ID=\"1080p60\",IVS-NAME=\"1080p60\",IVS-VARIANT-SOURCE=\"source\"\n\
https://cdn.example/1080.m3u8\n";

    #[test]
    fn parses_legacy() {
        let v = parse_master(LEGACY, "https://cdn.example/master.m3u8");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "1080p60");
        assert_eq!(v[0].group_id, "chunked");
    }

    #[test]
    fn parses_modern_media_less() {
        let v = parse_master(MODERN, "https://cdn.example/master.m3u8");
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].name, "1080p60");
        assert_eq!(v[1].group_id, "chunked");
    }

    #[test]
    fn selects_best_worst() {
        let v = parse_master(LEGACY, "https://cdn.example/master.m3u8");
        let (bi, _) = select_variant(&v, "best").unwrap();
        assert_eq!(v[bi].name, "1080p60");
        let (wi, _) = select_variant(&v, "worst").unwrap();
        assert_eq!(v[wi].name, "480p30");
    }
}
