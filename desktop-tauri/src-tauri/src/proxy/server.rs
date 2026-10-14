//! Local HTTP proxy server — port of the request-handling half of
//! `main/stream-proxy.ts`.
//!
//! Routes:
//! - `GET /stream?url=…&referer=…` and `/stream/playlist.m3u8` / `/stream/segment.ts`
//!   (both `?url=` and opaque `?u=` forms): pass-through proxy with `Referer`
//!   injection and HLS manifest rewriting.
//! - `GET /transcode/playlist.m3u8` / `/transcode/segment-<N>.ts`: VOD
//!   playlist synthesis + lazily transcoded segments.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::hls::{
    build_vod_playlist, convert_ass_to_webvtt, filter_hls_master_to_variant, is_ass_subtitle_url,
    is_hls_playlist_url, is_webvtt_subtitle_url, parse_byte_range, rewrite_hls_manifest,
};
use super::state::{ProxyState, SegmentWaitError, CHROME_UA, HLS_SEGMENT_DURATION_SECONDS};
use super::transcode;

#[derive(Debug, Deserialize)]
struct StreamQuery {
    url: Option<String>,
    u: Option<String>,
    referer: Option<String>,
    variant: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TranscodeQuery {
    url: Option<String>,
    referer: Option<String>,
    variant: Option<String>,
}

const CORS_HEADERS: [(&str, &str); 3] = [
    ("Access-Control-Allow-Origin", "*"),
    ("Access-Control-Allow-Methods", "GET,OPTIONS"),
    (
        "Access-Control-Allow-Headers",
        "Range,Accept,Content-Type,Origin,Referer,User-Agent",
    ),
];

fn cors_headers() -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in CORS_HEADERS {
        map.insert(k, v.parse().unwrap());
    }
    map
}

/// Headers that must not be forwarded from the upstream (hop-by-hop,
/// decode lies, CDN CORS that would override ours).
fn drop_upstream_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "transfer-encoding"
            | "connection"
            | "keep-alive"
            | "content-length"
            | "content-encoding"
            | "access-control-allow-origin"
            | "access-control-allow-credentials"
            | "access-control-allow-headers"
            | "access-control-allow-methods"
            | "access-control-expose-headers"
            | "access-control-max-age"
    )
}

fn is_hls_manifest_response(target_url: &str, content_type: Option<&str>) -> bool {
    if is_hls_playlist_url(target_url) {
        return true;
    }
    let ct = content_type.unwrap_or("").to_lowercase();
    ct.contains("application/vnd.apple.mpegurl") || ct.contains("application/x-mpegurl")
}

pub fn router(state: ProxyState) -> axum::Router {
    axum::Router::new()
        .route("/stream", get(handle_stream_passthrough))
        .route("/stream/*rest", get(handle_stream_passthrough))
        .route("/transcode/playlist.m3u8", get(handle_playlist))
        .route("/transcode/*rest", get(handle_transcode_rest))
        .fallback(options_fallback)
        .with_state(state)
}

async fn options_fallback(method: axum::http::Method) -> Response {
    if method == axum::http::Method::OPTIONS {
        (StatusCode::NO_CONTENT, cors_headers(), Body::empty()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Not found").into_response()
    }
}

pub async fn serve(state: ProxyState) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("proxy bind");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    state.set_port(port);
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .ok();
    });
    port
}

// ---------------------------------------------------------------------------
// passthrough
// ---------------------------------------------------------------------------

async fn handle_stream_passthrough(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Response {
    // Resolve target: opaque ?u= id or explicit ?url=.
    let (target_url, referer) = if let Some(id) = query.u.as_deref() {
        match state.resolve_upstream(id) {
            Some(t) => (t.url, t.referer),
            None => {
                return (StatusCode::NOT_FOUND, "Unknown stream target").into_response();
            }
        }
    } else if let Some(url) = query.url.clone() {
        (url, query.referer.clone())
    } else {
        let missing_opaque = query.u.is_some();
        return (
            if missing_opaque {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            },
            if missing_opaque {
                "Unknown stream target"
            } else {
                "Missing url parameter"
            },
        )
            .into_response();
    };
    // TODO(provider phase): normalizeStreamReferer via registered upstream handlers.
    let referer = referer.filter(|r| !r.trim().is_empty());
    let variant = query.variant.clone().unwrap_or_default();

    let mut req = state.http.get(&target_url);
    req = req.header(header::USER_AGENT, CHROME_UA);
    if let Some(accept) = headers.get(header::ACCEPT) {
        req = req.header(header::ACCEPT, accept.clone());
    } else {
        req = req.header(header::ACCEPT, "*/*");
    }
    if let Some(ref r) = referer {
        req = req.header(header::REFERER, r.clone());
        if let Ok(origin) = origin_of(r) {
            req = req.header(header::ORIGIN, origin);
        }
    }
    if let Some(range) = headers.get(header::RANGE) {
        req = req.header(header::RANGE, range.clone());
    }

    let upstream = match req.send().await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Proxy error: {e}"),
            )
                .into_response();
        }
    };
    let status = upstream.status();
    if status.as_u16() >= 400 {
        return (status, format!("Upstream error: {status}")).into_response();
    }

    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    // HLS playlists: filter master to one variant, rewrite to opaque proxy URLs.
    if is_hls_manifest_response(&target_url, content_type.as_deref()) {
        let body = match upstream.text().await {
            Ok(b) => b,
            Err(e) => return (StatusCode::BAD_GATEWAY, format!("Proxy error: {e}")).into_response(),
        };
        if !body.trim_start().starts_with("#EXTM3U") {
            return (
                StatusCode::BAD_GATEWAY,
                "Upstream playlist was not valid HLS",
            )
                .into_response();
        }
        let mut manifest = body;
        if manifest.contains("#EXT-X-STREAM-INF") {
            manifest = filter_hls_master_to_variant(&manifest, Some(&variant).filter(|v| !v.trim().is_empty()).map(String::as_str), 720);
        }
        let base = state.base_url();
        let rewritten = rewrite_hls_manifest(
            &manifest,
            &target_url,
            "/stream/playlist.m3u8",
            "/stream/segment.ts",
            &base,
            |absolute| {
                let effective = referer.clone().unwrap_or_else(|| target_url.clone());
                state.register_upstream(absolute, Some(&effective))
            },
        );
        let mut map = cors_headers();
        map.insert(
            header::CONTENT_TYPE,
            "application/vnd.apple.mpegurl; charset=utf-8".parse().unwrap(),
        );
        let code = if status == StatusCode::PARTIAL_CONTENT {
            StatusCode::PARTIAL_CONTENT
        } else {
            status
        };
        return (code, map, rewritten).into_response();
    }

    // ASS softsubs → WebVTT for native <track> support.
    if is_ass_subtitle_url(&target_url) {
        let body = match upstream.text().await {
            Ok(b) => b,
            Err(e) => return (StatusCode::BAD_GATEWAY, format!("Proxy error: {e}")).into_response(),
        };
        let mut map = cors_headers();
        map.insert(
            header::CONTENT_TYPE,
            "text/vtt; charset=utf-8".parse().unwrap(),
        );
        return (StatusCode::OK, map, convert_ass_to_webvtt(&body)).into_response();
    }

    let mut map = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if drop_upstream_header(name.as_str()) {
            continue;
        }
        map.append(name.clone(), value.clone());
    }
    if is_webvtt_subtitle_url(&target_url) {
        map.insert(
            header::CONTENT_TYPE,
            "text/vtt; charset=utf-8".parse().unwrap(),
        );
    }
    for (k, v) in cors_headers() {
        if let Some(k) = k {
            map.insert(k, v);
        }
    }
    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream);
    (status, map, body).into_response()
}

fn origin_of(referer: &str) -> Result<String, ()> {
    let url = url::Url::parse(referer).map_err(|_| ())?;
    Ok(format!(
        "{}://{}",
        url.scheme(),
        url.host_str().ok_or(())?
    ))
}

// ---------------------------------------------------------------------------
// transcode routes
// ---------------------------------------------------------------------------

async fn handle_playlist(
    State(state): State<ProxyState>,
    Query(query): Query<TranscodeQuery>,
) -> Response {
    let (Some(target_url), referer, variant) = (query.url.clone(), query.referer.clone(), query.variant.clone()) else {
        return (StatusCode::BAD_REQUEST, "Missing url parameter").into_response();
    };
    let input_url = build_local_proxy_input_url(&state, &target_url, referer.as_deref(), variant.as_deref());
    let key = match transcode::ensure_hls_session(
        &state,
        &input_url,
        &target_url,
        referer.as_deref(),
        variant.as_deref(),
    )
    .await
    {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };
    let (segment_count, target_url_c, referer_c, variant_c) = match state.with_inner(|i| {
        i.sessions.get(&key).map(|s| {
            (
                s.segment_count,
                s.target_url.clone(),
                s.referer.clone(),
                variant.clone(),
            )
        })
    }) {
        Some(v) => v,
        None => return (StatusCode::BAD_GATEWAY, "session gone").into_response(),
    };
    let _ = target_url_c;
    let body = build_vod_playlist(
        segment_count,
        HLS_SEGMENT_DURATION_SECONDS as u32,
        &target_url,
        referer_c.as_deref(),
        variant_c.as_deref(),
        &state.base_url(),
    );
    let mut map = cors_headers();
    map.insert(
        header::CONTENT_TYPE,
        "application/vnd.apple.mpegurl; charset=utf-8".parse().unwrap(),
    );
    map.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    (StatusCode::OK, map, body).into_response()
}

async fn handle_transcode_rest(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    axum::extract::Path(rest): axum::extract::Path<String>,
    Query(query): Query<TranscodeQuery>,
) -> Response {
    let seg: u32 = match rest.strip_prefix("segment-").and_then(|s| s.strip_suffix(".ts")).and_then(|n| n.parse().ok()) {
        Some(n) => n,
        None => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let (Some(target_url), referer, variant) = (query.url.clone(), query.referer.clone(), query.variant.clone()) else {
        return (StatusCode::BAD_REQUEST, "Missing url parameter").into_response();
    };
    let input_url = build_local_proxy_input_url(&state, &target_url, referer.as_deref(), variant.as_deref());
    let key = match transcode::ensure_hls_session(
        &state,
        &input_url,
        &target_url,
        referer.as_deref(),
        variant.as_deref(),
    )
    .await
    {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };
    let segment_count = match state.with_inner(|i| i.sessions.get(&key).map(|s| s.segment_count)) {
        Some(c) => c,
        None => return (StatusCode::BAD_GATEWAY, "session gone").into_response(),
    };
    if seg >= segment_count {
        return (StatusCode::NOT_FOUND, "Segment out of range").into_response();
    }
    let path = match transcode::ensure_segment_ready(&state, &key, seg).await {
        Ok(p) => p,
        Err(SegmentWaitError::PastEof(_)) => {
            return (StatusCode::GONE, format!("Segment {seg} is past the end of the stream"))
                .into_response();
        }
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    serve_segment_file(&path, headers.get(header::RANGE).and_then(|v| v.to_str().ok())).await
}

async fn serve_segment_file(path: &std::path::Path, range: Option<&str>) -> Response {
    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::NOT_FOUND, "Segment not found").into_response(),
    };
    let size = meta.len();
    let mut map = cors_headers();
    map.insert("Accept-Ranges", "bytes".parse().unwrap());
    map.insert(header::CONTENT_TYPE, "video/mp2t".parse().unwrap());
    map.insert(
        header::CACHE_CONTROL,
        "public, max-age=3600".parse().unwrap(),
    );

    let open = || tokio::fs::File::open(path);
    match parse_byte_range(range) {
        None => {
            let file = match open().await {
                Ok(f) => f,
                Err(_) => return (StatusCode::NOT_FOUND, "Segment not found").into_response(),
            };
            map.insert(header::CONTENT_LENGTH, size.to_string().parse().unwrap());
            let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
            (StatusCode::OK, map, body).into_response()
        }
        Some((start, end)) => {
            let end = end.unwrap_or_else(|| size.saturating_sub(1));
            if start > end || end >= size {
                map.insert(
                    header::CONTENT_RANGE,
                    format!("bytes */{size}").parse().unwrap(),
                );
                return (StatusCode::RANGE_NOT_SATISFIABLE, map, Body::empty()).into_response();
            }
            let mut file = match open().await {
                Ok(f) => f,
                Err(_) => return (StatusCode::NOT_FOUND, "Segment not found").into_response(),
            };
            if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return (StatusCode::BAD_GATEWAY, "Seek failed").into_response();
            }
            let limited = file.take(end - start + 1);
            map.insert(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{size}").parse().unwrap(),
            );
            map.insert(
                header::CONTENT_LENGTH,
                (end - start + 1).to_string().parse().unwrap(),
            );
            let body = Body::from_stream(tokio_util::io::ReaderStream::new(limited));
            (StatusCode::PARTIAL_CONTENT, map, body).into_response()
        }
    }
}

/// Opaque-id input URL so ffmpeg never sees nested CDN hosts in `?url=`.
fn build_local_proxy_input_url(    state: &ProxyState,
    target_url: &str,
    referer: Option<&str>,
    variant: Option<&str>,
) -> String {
    let id = state.register_upstream(target_url, referer);
    let mut params = vec![("u".to_string(), id)];
    if let Some(v) = variant.map(str::trim).filter(|s| !s.is_empty()) {
        params.push(("variant".to_string(), v.to_string()));
    }
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}={}",
                url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
            )
        })
        .collect();
    format!("{}/stream/playlist.m3u8?{}", state.base_url(), query.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720\nhi/index.m3u8\n";
    const MEDIA: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nseg0.ts\n#EXTINF:6.0,\nseg1.ts\n#EXT-X-ENDLIST\n";

    use super::support::range_aware_bytes;

    async fn mock_upstream() -> String {
        let app = axum::Router::new()
            .route(
                "/master.m3u8",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
                        MASTER,
                    )
                }),
            )
            .route(
                "/hi/index.m3u8",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
                        MEDIA,
                    )
                }),
            )
            .route(
                "/hi/seg0.ts",
                get(|headers: HeaderMap| async {
                    range_aware_bytes(headers, b"FAKESEGMENT".to_vec()).await
                }),
            )
            .route(
                "/subs/dialogue.ass",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/plain")],
                        "[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hi",
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service()).await.ok();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    async fn proxy_server() -> (ProxyState, String) {
        let state = ProxyState::new();
        let port = serve(state.clone()).await;
        assert!(port > 0);
        (state, format!("http://127.0.0.1:{port}"))
    }

    #[tokio::test]
    async fn passthrough_rewrites_master_to_opaque_urls() {
        let upstream = mock_upstream().await;
        let (_state, base) = proxy_server().await;
        let client = reqwest::Client::new();

        let master_url = format!("{upstream}/master.m3u8");
        let body = client
            .get(format!("{base}/stream?url={master_url}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // Filtered to single (720p) variant and rewritten to opaque proxy URLs.
        assert!(body.contains("/stream/playlist.m3u8?u=t"), "got:\n{body}");
        assert!(!body.contains("low/index.m3u8"));
        assert!(!body.contains("cdn") && !body.contains("127.0.0.1:") || body.contains(&base));
        assert!(!body.contains(&upstream));
    }

    #[tokio::test]
    async fn nested_playlist_and_segment_resolve() {
        let upstream = mock_upstream().await;
        let (_state, base) = proxy_server().await;
        let client = reqwest::Client::new();

        let master_url = format!("{upstream}/master.m3u8");
        let master = client
            .get(format!("{base}/stream?url={master_url}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // Follow the single rewritten variant link.
        let variant_line = master
            .lines()
            .find(|l| l.contains("/stream/playlist.m3u8?u="))
            .expect("rewritten variant")
            .to_string();
        let media = client.get(&variant_line).send().await.unwrap().text().await.unwrap();
        assert!(media.contains("/stream/segment.ts?u="), "got:\n{media}");
        let seg_line = media
            .lines()
            .find(|l| l.contains("/stream/segment.ts?u="))
            .expect("rewritten segment")
            .to_string();
        let seg = client.get(&seg_line).send().await.unwrap();
        assert_eq!(seg.status(), 200);
        assert_eq!(seg.bytes().await.unwrap().as_ref(), b"FAKESEGMENT");
    }

    #[tokio::test]
    async fn ass_subtitles_convert_to_vtt() {
        let upstream = mock_upstream().await;
        let (_state, base) = proxy_server().await;
        let client = reqwest::Client::new();
        let sub_url = format!("{upstream}/subs/dialogue.ass");
        let res = client
            .get(format!("{base}/stream?url={sub_url}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.headers()["content-type"], "text/vtt; charset=utf-8");
        let body = res.text().await.unwrap();
        assert!(body.starts_with("WEBVTT"));
        assert!(body.contains("00:00:01.000 --> 00:00:02.000"));
    }

    #[tokio::test]
    async fn passthrough_preserves_mp4_bytes() {
        let upstream = mock_upstream().await;
        let (_state, base) = proxy_server().await;
        let client = reqwest::Client::new();
        // 292KB fixture-equivalent: fetch mp4 through proxy, compare length + prefix.
        let direct = client
            .get(format!("{upstream}/hi/seg0.ts"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let via_proxy = client
            .get(format!("{base}/stream?url={upstream}/hi/seg0.ts"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(direct.len(), via_proxy.len());
        assert_eq!(direct.as_ref(), via_proxy.as_ref());
    }

    #[tokio::test]
    async fn transcode_without_ffmpeg_is_502() {
        let (_state, base) = proxy_server().await;
        let client = reqwest::Client::new();
        let res = client
            .get(format!("{base}/transcode/playlist.m3u8?url=http://example.com/x.m3u8"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 502);
    }
}

#[cfg(test)]
mod support {
    use super::*;

    /// Serve bytes honoring `Range` like a real CDN (axum ignores Range by default).
    pub async fn range_aware_bytes(headers: HeaderMap, bytes: Vec<u8>) -> impl IntoResponse {
        let total = bytes.len() as u64;
        let base: HeaderMap = [(header::CONTENT_TYPE, "application/octet-stream")]
            .into_iter()
            .map(|(k, v)| (k, v.parse().unwrap()))
            .collect();
        let had_range = headers.contains_key(header::RANGE);
        let (start, end) = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| crate::proxy::hls::parse_byte_range(Some(v)))
            .map(|(s, e)| (s, e.unwrap_or_else(|| total.saturating_sub(1))))
            .unwrap_or((0, total.saturating_sub(1)));
        if total == 0 || start > end || end >= total {
            return (StatusCode::RANGE_NOT_SATISFIABLE, base, Body::empty());
        }
        let mut map = base;
        map.insert("Accept-Ranges", "bytes".parse().unwrap());
        if had_range {
            map.insert(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{total}").parse().unwrap(),
            );
            (
                StatusCode::PARTIAL_CONTENT,
                map,
                Body::from(bytes[start as usize..=end as usize].to_vec()),
            )
        } else {
            (StatusCode::OK, map, Body::from(bytes))
        }
    }
}

#[cfg(test)]
mod transcode_tests {
    use super::support::range_aware_bytes;
    use super::*;
    use std::path::PathBuf;

    fn sidecar_ffmpeg() -> String {
        for name in [
            "ffmpeg-x86_64-unknown-linux-gnu",
            "ffmpeg-x86_64-pc-windows-msvc.exe",
        ] {
            let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(name);
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
        panic!("no ffmpeg sidecar in src-tauri/binaries (run npm run download-ffmpeg)");
    }

    #[tokio::test]
    async fn transcode_mp4_end_to_end() {
        // 13s fixture → floor(13/6) = 2 segments.
        let app = axum::Router::new().route(
            "/src.mp4",
            get(|headers: HeaderMap| async move {
                range_aware_bytes(headers, tokio::fs::read("/tmp/openmedia-test-src.mp4").await.unwrap()).await
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service()).await.ok();
        });

        let mut state = ProxyState::new();
        state.ffmpeg_path = Some(sidecar_ffmpeg());
        let port = serve(state.clone()).await;
        let base = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::new();
        let target = format!("{upstream}/src.mp4");

        let playlist = client
            .get(format!("{base}/transcode/playlist.m3u8?url={target}"))
            .send()
            .await
            .unwrap();
        let pstatus = playlist.status();
        let body = playlist.text().await.unwrap();
        assert_eq!(pstatus, 200, "playlist body: {body}");
        assert!(body.contains("#EXT-X-ENDLIST"), "got:\n{body}");
        let seg_count = body.matches("#EXTINF:").count();
        assert_eq!(seg_count, 2, "got:\n{body}");

        let seg0 = client
            .get(format!("{base}/transcode/segment-0.ts?url={target}"))
            .send()
            .await
            .unwrap();
        let seg_status = seg0.status();
        let seg_ctype = seg0
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let seg_body = seg0.bytes().await.unwrap();
        if seg_status != 200 {
            let prog = state.with_inner(|i| {
                i.sessions
                    .values()
                    .map(|s| {
                        (
                            s.key.clone(),
                            s.sequential_job.as_ref().map(|j| {
                                (j.start_segment, j.exit_code, j.clean_exit, j.stderr_tail.clone())
                            }),
                        )
                    })
                    .collect::<Vec<_>>()
            });
            panic!(
                "seg0 status {seg_status} body: {} sessions: {prog:?}",
                String::from_utf8_lossy(&seg_body)
            );
        }
        assert_eq!(seg_status, 200);
        assert_eq!(seg_ctype, "video/mp2t");
        // (seg_body already consumed above)
        let bytes = seg_body;
        assert!(bytes.len() > 10_000, "seg0 too small: {}", bytes.len());
        // MPEG-TS sync byte.
        assert_eq!(bytes[0], 0x47);

        // Out-of-range segment → 404.
        let bad = client
            .get(format!("{base}/transcode/segment-9.ts?url={target}"))
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 404);
    }
}
