//! HLS transcode engine — port of the session/job half of
//! `main/stream-proxy.ts`.
//!
//! One `HlsSession` per (target URL, variant). A sequential ffmpeg job
//! transcodes forward from a start segment; HTTP segment handlers wait on
//! per-segment oneshot awaiters. Forward seeks restart the job at the
//! requested segment; clean exits past the produced range are reported as
//! past-EOF (HTTP 410) instead of retried forever.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

use super::hls::transcode_cache_key;
use super::state::{
    AwaiterQueue, HlsSession, ProxyState, SegmentWaitError, SequentialJob, TranscodeProgress,
    ENSURE_SEGMENT_MAX_RESTART_RETRIES, FIRST_SEGMENT_WAIT_MS, HLS_SEGMENT_DURATION_SECONDS,
    PLAYLIST_PROBE_TIMEOUT_MS, PROBE_TIMEOUT_MS, SAME_START_FAILURE_COOLDOWN_MS,
    SEGMENT_RESTART_FORWARD_TOLERANCE, SEGMENT_WAIT_TIMEOUT_MS,
};

// ---------------------------------------------------------------------------
// session lifecycle
// ---------------------------------------------------------------------------

pub fn session_key(target_url: &str, variant: Option<&str>) -> String {
    transcode_cache_key(target_url, variant)
}

/// Get or create the transcode session (probes duration on first use).
pub async fn ensure_hls_session(
    state: &ProxyState,
    input_url: &str,
    target_url: &str,
    referer: Option<&str>,
    variant: Option<&str>,
) -> Result<String, String> {
    let key = session_key(target_url, variant);

    if state.with_inner(|i| i.sessions.contains_key(&key)) {
        return Ok(key);
    }
    let init_mutex = state.with_inner(|i| {
        i.session_inits
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    });
    let _guard = init_mutex.lock().await;
    if state.with_inner(|i| i.sessions.contains_key(&key)) {
        state.with_inner(|i| {
            i.session_inits.remove(&key);
        });
        return Ok(key);
    }

    let result = init_session(state, &key, input_url, target_url, referer, variant).await;
    state.with_inner(|i| {
        i.session_inits.remove(&key);
    });
    result?;
    Ok(key)
}

async fn init_session(
    state: &ProxyState,
    key: &str,
    input_url: &str,
    target_url: &str,
    referer: Option<&str>,
    variant: Option<&str>,
) -> Result<(), String> {
    if state.ffmpeg_path.is_none() {
        return Err("ffmpeg binary unavailable".to_string());
    }
    state.set_progress(
        key,
        TranscodeProgress {
            state: "running",
            progress_percent: None,
            message: "Probing stream...".to_string(),
        },
    );

    let segment_dir = state.cache_dir.join(key);
    tokio::fs::create_dir_all(&segment_dir)
        .await
        .map_err(|e| format!("cache dir: {e}"))?;

    let mut hls_input = false;
    let duration = match probe_duration_from_hls_playlist(state, input_url, referer).await {
        Some(d) if d > 0.0 => {
            hls_input = true;
            d
        }
        _ => probe_duration_seconds(state, input_url).await?,
    };
    if !duration.is_finite() || duration <= 0.0 {
        state.set_progress(
            key,
            TranscodeProgress {
                state: "error",
                progress_percent: None,
                message: "Stream has no known duration".to_string(),
            },
        );
        return Err("Could not determine stream duration".to_string());
    }
    // floor() so we never advertise a phantom trailing segment past EOF.
    let segment_count = ((duration / HLS_SEGMENT_DURATION_SECONDS as f64).floor() as u32).max(1);

    let mut available = std::collections::HashSet::new();
    for i in 0..segment_count {
        let p = segment_dir.join(format!("seg-{i}.ts"));
        if let Ok(meta) = tokio::fs::metadata(&p).await {
            if meta.is_file() && meta.len() > 0 {
                available.insert(i);
            }
        }
    }

    state.with_inner(|i| {
        i.sessions.insert(
            key.to_string(),
            HlsSession {
                key: key.to_string(),
                input_url: input_url.to_string(),
                target_url: target_url.to_string(),
                referer: referer.map(str::to_string),
                hls_input,
                duration_seconds: duration,
                segment_count,
                segment_dir,
                sequential_job: None,
                segments_available: available,
                segments_unavailable: std::collections::HashSet::new(),
                awaiters: std::collections::HashMap::new(),
                last_failed_start_segment: None,
                last_failed_at: None,
            },
        );
    });

    update_session_progress(state, key, "Building stream cache...");

    let (have, total) = state.with_inner(|i| {
        let s = i.sessions.get(key).unwrap();
        (s.segments_available.len() as u32, s.segment_count)
    });
    if have < total {
        let first_missing = state.with_inner(|i| {
            let s = i.sessions.get(key).unwrap();
            (0..s.segment_count)
                .find(|n| !s.segments_available.contains(n))
                .unwrap_or(s.segment_count)
        });
        start_sequential_job(state, key, first_missing)?;
    } else {
        state.set_progress(
            key,
            TranscodeProgress {
                state: "done",
                progress_percent: Some(100.0),
                message: "Stream cache ready".to_string(),
            },
        );
    }
    let _ = variant;
    Ok(())
}

fn update_session_progress(state: &ProxyState, key: &str, message: &str) {
    state.with_inner(|i| {
        if let Some(s) = i.sessions.get(key) {
            let ratio = s.segments_available.len() as f64 / s.segment_count.max(1) as f64;
            let done = s.segments_available.len() as u32 >= s.segment_count;
            i.progress.insert(
                key.to_string(),
                TranscodeProgress {
                    state: if done { "done" } else { "running" },
                    progress_percent: if done {
                        Some(100.0)
                    } else {
                        Some((ratio * 100.0 * 10.0).floor() / 10.0)
                    },
                    message: if done {
                        "Stream cache ready".to_string()
                    } else {
                        message.to_string()
                    },
                },
            );
        }
    });
}

/// Stop all background ffmpeg processes (e.g. on app quit).
pub fn shutdown_transcode_jobs(state: &ProxyState) {
    let keys: Vec<String> = state.with_inner(|i| i.sessions.keys().cloned().collect());
    for key in keys {
        cancel_session(state, &key);
    }
}

/// Stop the background transcoder for one stream.
pub fn cancel_transcoded_stream(state: &ProxyState, target_url: &str, variant: Option<&str>) {
    let key = session_key(target_url, variant);
    if !state.with_inner(|i| i.sessions.contains_key(&key)) {
        state.set_progress(
            &key,
            TranscodeProgress {
                state: "idle",
                progress_percent: None,
                message: "Cancelled".to_string(),
            },
        );
        return;
    }
    cancel_session(state, &key);
}

fn cancel_session(state: &ProxyState, key: &str) {
    let awaiters: Vec<(u32, AwaiterQueue)> =
        state.with_inner(|i| {
            if let Some(s) = i.sessions.get_mut(key) {
                if let Some(job) = s.sequential_job.as_mut() {
                    job.stopped = true;
                    let _ = job.child.start_kill();
                }
                s.sequential_job = None;
                let mut out = Vec::new();
                for (seg, list) in s.awaiters.drain() {
                    out.push((seg, list));
                }
                out
            } else {
                Vec::new()
            }
        });
    for (_, list) in awaiters {
        for w in list {
            let _ = w.send(Err(SegmentWaitError::Cancelled));
        }
    }
    state.set_progress(
        key,
        TranscodeProgress {
            state: "idle",
            progress_percent: None,
            message: "Cancelled".to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// duration probing
// ---------------------------------------------------------------------------

/// Sum #EXTINF from an HLS media playlist (follow master → first variant).
async fn probe_duration_from_hls_playlist(
    state: &ProxyState,
    playlist_url: &str,
    referer: Option<&str>,
) -> Option<f64> {
    let mut req = state
        .http
        .get(playlist_url)
        .timeout(Duration::from_millis(PLAYLIST_PROBE_TIMEOUT_MS));
    if let Some(r) = referer.filter(|r| !r.trim().is_empty()) {
        req = req.header(reqwest::header::REFERER, r.to_string());
    }
    let text = req.send().await.ok()?;
    if !text.status().is_success() {
        return None;
    }
    let mut text = text.text().await.ok()?;
    if !text.trim_start().starts_with("#EXTM3U") {
        return None;
    }
    if text.contains("#EXT-X-STREAM-INF") {
        let media_url = pick_first_variant_url(&text, playlist_url)?;
        let mut media_req = state
            .http
            .get(&media_url)
            .timeout(Duration::from_millis(PLAYLIST_PROBE_TIMEOUT_MS));
        if let Some(r) = referer.filter(|r| !r.trim().is_empty()) {
            media_req = media_req.header(reqwest::header::REFERER, r.to_string());
        }
        let media = media_req.send().await.ok()?;
        if !media.status().is_success() {
            return None;
        }
        text = media.text().await.ok()?;
        if !text.trim_start().starts_with("#EXTM3U") {
            return None;
        }
    }
    let mut total = 0.0;
    let mut count = 0u32;
    for line in text.lines() {
        let rest = line.strip_prefix("#EXTINF:")?;
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let seconds: f64 = num.parse().ok()?;
        if !seconds.is_finite() || seconds < 0.0 {
            continue;
        }
        total += seconds;
        count += 1;
    }
    if count == 0 || total <= 0.0 {
        return None;
    }
    Some(total)
}

fn pick_first_variant_url(master: &str, base_url: &str) -> Option<String> {
    let lines: Vec<&str> = master.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with("#EXT-X-STREAM-INF") {
            if let Some(next) = lines.get(idx + 1).map(|l| l.trim()) {
                if !next.is_empty() && !next.starts_with('#') {
                    return Some(super::hls::to_absolute_url(next, base_url));
                }
            }
        }
    }
    None
}

/// Parse `Duration: HH:MM:SS.ms` from `ffmpeg -i <url>` stderr (no ffprobe needed).
async fn probe_duration_seconds(state: &ProxyState, input_url: &str) -> Result<f64, String> {
    let ffmpeg = state
        .ffmpeg_path
        .clone()
        .ok_or_else(|| "ffmpeg binary unavailable".to_string())?;
    // HLS-demuxer options first (the common case, 1:1 with upstream); retry
    // bare when ffmpeg rejects them (non-HLS inputs like mp4).
    let hls_args = [
        "-protocol_whitelist",
        "file,http,https,tcp,tls,crypto,data",
        "-allowed_extensions",
        "ALL",
    ];
    match probe_duration_once(&ffmpeg, input_url, &hls_args).await {
        Ok(d) => Ok(d),
        Err(e) if e.contains("not found") => probe_duration_once(&ffmpeg, input_url, &[]).await,
        Err(e) => Err(e),
    }
}

async fn probe_duration_once(
    ffmpeg: &str,
    input_url: &str,
    extra_input_args: &[&str],
) -> Result<f64, String> {
    let mut child = tokio::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "info"])
        .args(extra_input_args)
        .args(["-i", input_url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg spawn: {e}"))?;

    let mut stderr = child.stderr.take();
    let reader = async move {
        let mut buf = Vec::new();
        if let Some(ref mut err) = stderr {
            // Bound the read; we only need the header.
            let _ = tokio::time::timeout(
                Duration::from_millis(PROBE_TIMEOUT_MS),
                err.take(65536).read_to_end(&mut buf),
            )
            .await;
        }
        buf
    };
    let buf = reader.await;
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

    let text = String::from_utf8_lossy(&buf);
    let re = regex::Regex::new(r"Duration:\s*(\d+):(\d{2}):(\d{2}(?:\.\d+)?)").unwrap();
    if let Some(cap) = re.captures(&text) {
        let h: f64 = cap[1].parse().unwrap_or(0.0);
        let m: f64 = cap[2].parse().unwrap_or(0.0);
        let s: f64 = cap[3].parse().unwrap_or(0.0);
        let total = h * 3600.0 + m * 60.0 + s;
        if total.is_finite() && total > 0.0 {
            return Ok(total);
        }
    }
    Err(format!(
        "ffmpeg duration probe failed: {}",
        last_chars(&text, 512)
    ))
}

fn last_chars(s: &str, n: usize) -> String {
    let total = s.chars().count();
    s.chars().skip(total.saturating_sub(n)).collect()
}

// ---------------------------------------------------------------------------
// sequential ffmpeg jobs
// ---------------------------------------------------------------------------

/// Start (or restart) the forward transcode job at `start_segment`.
pub fn start_sequential_job(
    state: &ProxyState,
    key: &str,
    start_segment: u32,
) -> Result<(), String> {
    let ffmpeg = state
        .ffmpeg_path
        .clone()
        .ok_or_else(|| "ffmpeg binary unavailable".to_string())?;

    // Snapshot session fields, fail out-of-range awaiters, stop previous job.
    let (input_url, hls_input, segment_dir, segment_count) = state.with_inner(|i| {
        let s = i
            .sessions
            .get_mut(key)
            .ok_or_else(|| "unknown session".to_string())?;
        let start = start_segment.min(s.segment_count.saturating_sub(1));
        // Fail awaiters the new job will never reach.
        let mut doomed = Vec::new();
        for (seg, list) in s.awaiters.extract_if(|seg, _| {
            *seg < start && !s.segments_available.contains(seg)
        }) {
            doomed.push((seg, list));
        }
        for (seg, list) in doomed {
            for w in list {
                let _ = w.send(Err(SegmentWaitError::TranscoderRestarted {
                    seg,
                    new_start: start,
                }));
            }
        }
        if let Some(job) = s.sequential_job.as_mut() {
            job.stopped = true;
            let _ = job.child.start_kill();
        }
        s.sequential_job = None;
        Ok::<_, String>((
            s.input_url.clone(),
            s.hls_input,
            s.segment_dir.clone(),
            s.segment_count,
        ))
    })?;
    let start_segment = start_segment.min(segment_count.saturating_sub(1));

    let start_time = start_segment as f64 * HLS_SEGMENT_DURATION_SECONDS as f64;
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-nostats".into(),
    ];
    if start_segment > 0 {
        args.push("-ss".into());
        args.push(format!("{start_time:.3}"));
    }
    if hls_input {
        args.push("-protocol_whitelist".into());
        args.push("file,http,https,tcp,tls,crypto,data".into());
        args.push("-allowed_extensions".into());
        args.push("ALL".into());
    }
    args.extend(
        [
            "-i",
            &input_url,
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-profile:a",
            "aac_low",
            "-b:a",
            "128k",
            "-ac",
            "2",
            "-ar",
            "48000",
            "-output_ts_offset",
            &format!("{start_time:.3}"),
            "-f",
            "hls",
            "-hls_time",
            &HLS_SEGMENT_DURATION_SECONDS.to_string(),
            "-hls_playlist_type",
            "vod",
            "-hls_segment_type",
            "mpegts",
            "-hls_list_size",
            "0",
            "-start_number",
            &start_segment.to_string(),
            "-hls_segment_filename",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.push(
        segment_dir
            .join("seg-%d.ts")
            .to_string_lossy()
            .into_owned(),
    );
    args.extend(
        [
            "-hls_flags",
            "independent_segments+temp_file+omit_endlist",
            "-y",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.push(
        segment_dir
            .join(".internal.m3u8")
            .to_string_lossy()
            .into_owned(),
    );

    // Borrow dance for the '-i <url>' entry pushed via &input_url above.
    let mut child = tokio::process::Command::new(&ffmpeg)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            fail_spawn_awaiters(state, key, &e.to_string());
            format!("ffmpeg spawn: {e}")
        })?;

    let stderr = child.stderr.take();
    let job = SequentialJob {
        child,
        start_segment,
        highest_completed_segment: start_segment as i64 - 1,
        stopped: false,
        stderr_tail: String::new(),
        exit_code: None,
        clean_exit: false,
    };
    state.with_inner(|i| {
        if let Some(s) = i.sessions.get_mut(key) {
            s.sequential_job = Some(job);
        }
    });

    // Drain stderr in background (bounded tail for diagnostics).
    if let Some(stderr) = stderr {
        let state_clone = state.clone();
        let key_owned = key.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut buf = vec![0u8; 4096];
            use tokio::io::AsyncReadExt;
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                        state_clone.with_inner(|i| {
                            if let Some(s) = i.sessions.get_mut(&key_owned) {
                                if let Some(job) = s.sequential_job.as_mut() {
                                    job.stderr_tail.push_str(&text);
                                    if job.stderr_tail.len() > 5000 {
                                        job.stderr_tail =
                                            job.stderr_tail[job.stderr_tail.len() - 5000..].to_string();
                                    }
                                }
                            }
                        });
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Filesystem poll: with temp_file the muxer atomically renames
    // seg-N.ts.tmp → seg-N.ts when complete, so any visible file is servable.
    let poll_state = state.clone();
    let poll_key = key.to_string();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(200));
        loop {
            interval.tick().await;
            let alive = poll_state.with_inner(|i| {
                i.sessions
                    .get(&poll_key)
                    .and_then(|s| s.sequential_job.as_ref())
                    .map(|j| !j.stopped)
                    .unwrap_or(false)
            });
            if !alive {
                break;
            }
            scan_segments_for_job(&poll_state, &poll_key).await;
            let done = poll_state.with_inner(|i| {
                i.sessions.get(&poll_key).map(|s| {
                    s.segments_available.len() as u32 >= s.segment_count
                })
                .unwrap_or(true)
            });
            if done {
                break;
            }
            // Detect child exit via try_wait.
            let exited = poll_state.with_inner(|i| {
                if let Some(s) = i.sessions.get_mut(&poll_key) {
                    if let Some(job) = s.sequential_job.as_mut() {
                        if job.stopped {
                            return true;
                        }
                        match job.child.try_wait() {
                            Ok(Some(status)) => {
                                job.exit_code = status.code();
                                job.clean_exit = status.success();
                                true
                            }
                            Ok(None) => false,
                            Err(_) => true,
                        }
                    } else {
                        true
                    }
                } else {
                    true
                }
            });
            if exited {
                scan_segments_for_job(&poll_state, &poll_key).await;
                finalize_awaiters_after_job_exit(&poll_state, &poll_key);
                poll_state.with_inner(|i| {
                    if let Some(s) = i.sessions.get_mut(&poll_key) {
                        s.sequential_job = None;
                    }
                });
                break;
            }
        }
    });

    Ok(())
}

fn fail_spawn_awaiters(state: &ProxyState, key: &str, message: &str) {
    let pending: Vec<AwaiterQueue> =
        state.with_inner(|i| {
            let Some(s) = i.sessions.get_mut(key) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for (seg, list) in s.awaiters.extract_if(|seg, _| !s.segments_available.contains(seg)) {
                let _ = seg;
                out.push(list);
            }
            out
        });
    for list in pending {
        for w in list {
            let _ = w.send(Err(SegmentWaitError::Failed(format!(
                "Transcoder failed to start: {message}"
            ))));
        }
    }
}

async fn scan_segments_for_job(state: &ProxyState, key: &str) {
    let (segment_dir, segment_count, scan_from) = match state.with_inner(|i| {
        i.sessions.get(key).map(|s| {
            let from = s
                .sequential_job
                .as_ref()
                .map(|j| (j.highest_completed_segment + 1).max(j.start_segment as i64))
                .unwrap_or(0)
                .max(0) as u32;
            (s.segment_dir.clone(), s.segment_count, from)
        })
    }) {
        Some(v) => v,
        None => return,
    };
    let mut newly: Vec<u32> = Vec::new();
    let mut probe = scan_from;
    while probe < segment_count {
        let p = segment_dir.join(format!("seg-{probe}.ts"));
        let ok = tokio::fs::metadata(&p)
            .await
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);
        if !ok {
            break;
        }
        newly.push(probe);
        probe += 1;
    }
    if newly.is_empty() {
        return;
    }
    let notify: Vec<(u32, AwaiterQueue)> =
        state.with_inner(|i| {
            let mut notify = Vec::new();
            if let Some(s) = i.sessions.get_mut(key) {
                for seg in &newly {
                    if let Some(job) = s.sequential_job.as_mut() {
                        job.highest_completed_segment =
                            job.highest_completed_segment.max(*seg as i64);
                    }
                    s.segments_available.insert(*seg);
                    if let Some(list) = s.awaiters.remove(seg) {
                        notify.push((*seg, list));
                    }
                }
            }
            notify
        });
    for (_, list) in notify {
        for w in list {
            let _ = w.send(Ok(()));
        }
    }
    update_session_progress(state, key, "Building stream cache...");
}

fn finalize_awaiters_after_job_exit(state: &ProxyState, key: &str) {
    let pending: Vec<(u32, AwaiterQueue, JobExitSummary)> =
        state.with_inner(|i| {
            let Some(s) = i.sessions.get_mut(key) else {
                return Vec::new();
            };
            let Some(job) = s.sequential_job.as_ref() else {
                return Vec::new();
            };
            if job.stopped {
                return Vec::new();
            }
            let summary = JobExitSummary {
                clean_exit: job.clean_exit,
                exit_code: job.exit_code,
                highest_completed: job.highest_completed_segment,
                start_segment: job.start_segment,
                stderr_tail: job.stderr_tail.clone(),
            };
            if !summary.clean_exit && summary.highest_completed < summary.start_segment as i64 {
                s.last_failed_start_segment = Some(summary.start_segment);
                s.last_failed_at = Some(Instant::now());
            } else {
                s.last_failed_start_segment = None;
                s.last_failed_at = None;
            }
            let mut out = Vec::new();
            for (seg, list) in s.awaiters.drain() {
                if s.segments_available.contains(&seg) {
                    continue;
                }
                out.push((seg, list, summary.clone()));
            }
            out
        });
    for (seg, list, summary) in pending {
        let past_eof = summary.clean_exit
            && seg as i64 > summary.highest_completed
            && summary.highest_completed >= summary.start_segment as i64 - 1;
        if past_eof {
            state.with_inner(|i| {
                if let Some(s) = i.sessions.get_mut(key) {
                    s.segments_unavailable.insert(seg);
                }
            });
            for w in list {
                let _ = w.send(Err(SegmentWaitError::PastEof(seg)));
            }
            continue;
        }
        let detail = {
            let d = last_chars(&summary.stderr_tail, 1024);
            if d.trim().is_empty() {
                "<no stderr>".to_string()
            } else {
                d
            }
        };
        for w in list {
            let _ = w.send(Err(SegmentWaitError::Failed(format!(
                "Transcoder exited (code={:?}) before producing segment {seg}: {detail}",
                summary.exit_code
            ))));
        }
    }
}

#[derive(Clone)]
struct JobExitSummary {
    clean_exit: bool,
    exit_code: Option<i32>,
    highest_completed: i64,
    start_segment: u32,
    stderr_tail: String,
}

// ---------------------------------------------------------------------------
// segment readiness (HTTP handler entry point)
// ---------------------------------------------------------------------------

/// Wait until `seg` is on disk, spawning/restarting the job as needed.
/// Returns the segment file path.
pub async fn ensure_segment_ready(
    state: &ProxyState,
    key: &str,
    seg: u32,
) -> Result<std::path::PathBuf, SegmentWaitError> {
    let segment_path = |state: &ProxyState| {
        state.with_inner(|i| {
            i.sessions
                .get(key)
                .map(|s| s.segment_dir.join(format!("seg-{seg}.ts")))
        })
    };

    let mut restarts = 0u32;
    loop {
        // Fail fast on sticky past-EOF.
        if state.with_inner(|i| {
            i.sessions
                .get(key)
                .map(|s| s.segments_unavailable.contains(&seg))
                .unwrap_or(false)
        }) {
            return Err(SegmentWaitError::PastEof(seg));
        }
        // Cheap re-stat in case another request just produced it.
        let known = state.with_inner(|i| {
            i.sessions
                .get(key)
                .map(|s| s.segments_available.contains(&seg))
                .unwrap_or(false)
        });
        if !known {
            if let Some(path) = segment_path(state) {
                if let Ok(meta) = tokio::fs::metadata(&path).await {
                    if meta.is_file() && meta.len() > 0 {
                        state.with_inner(|i| {
                            if let Some(s) = i.sessions.get_mut(key) {
                                s.segments_available.insert(seg);
                            }
                        });
                    }
                }
            }
        }
        if state.with_inner(|i| {
            i.sessions
                .get(key)
                .map(|s| s.segments_available.contains(&seg))
                .unwrap_or(false)
        }) {
            return segment_path(state).ok_or(SegmentWaitError::Failed("session gone".into()));
        }

        let reachable = state.with_inner(|i| {
            i.sessions
                .get(key)
                .and_then(|s| s.sequential_job.as_ref())
                .map(|job| {
                    !job.stopped
                        && seg >= job.start_segment
                        && seg as i64
                            <= job.highest_completed_segment
                                + 1
                                + SEGMENT_RESTART_FORWARD_TOLERANCE as i64
                })
                .unwrap_or(false)
        });
        if !reachable {
            let cooldown_hit = state.with_inner(|i| {
                i.sessions.get(key).map(|s| {
                    s.last_failed_start_segment == Some(seg)
                        && s.last_failed_at
                            .map(|t| t.elapsed() < Duration::from_millis(SAME_START_FAILURE_COOLDOWN_MS))
                            .unwrap_or(false)
                })
                .unwrap_or(false)
            });
            if cooldown_hit {
                return Err(SegmentWaitError::Failed(format!(
                    "Segment {seg} not retried: transcoder just failed at this position"
                )));
            }
            start_sequential_job(state, key, seg).map_err(SegmentWaitError::Failed)?;
        }

        match wait_for_segment(state, key, seg).await {
            Ok(()) => {
                return segment_path(state).ok_or(SegmentWaitError::Failed("session gone".into()));
            }
            Err(SegmentWaitError::PastEof(_)) => {
                return Err(SegmentWaitError::PastEof(seg));
            }
            Err(SegmentWaitError::TranscoderRestarted { .. }) => {
                restarts += 1;
                if restarts > ENSURE_SEGMENT_MAX_RESTART_RETRIES {
                    return Err(SegmentWaitError::Failed(format!(
                        "Segment {seg} cancelled by repeated transcoder restarts"
                    )));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

async fn wait_for_segment(
    state: &ProxyState,
    key: &str,
    seg: u32,
) -> Result<(), SegmentWaitError> {
    if state.with_inner(|i| {
        i.sessions
            .get(key)
            .map(|s| s.segments_available.contains(&seg))
            .unwrap_or(false)
    }) {
        return Ok(());
    }
    let (tx, rx) = oneshot::channel();
    state.with_inner(|i| {
        if let Some(s) = i.sessions.get_mut(key) {
            s.awaiters.entry(seg).or_default().push(tx);
        }
    });
    // Remove our awaiter if we time out (avoid leaking the slot).
    let state_clone = state.clone();
    let key_owned = key.to_string();
    match tokio::time::timeout(Duration::from_millis(SEGMENT_WAIT_TIMEOUT_MS), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(SegmentWaitError::Cancelled),
        Err(_) => {
            state_clone.with_inner(|i| {
                if let Some(s) = i.sessions.get_mut(&key_owned) {
                    if let Some(list) = s.awaiters.get_mut(&seg) {
                        // Can't identify our sender; drop the whole slot only if
                        // receivers are gone is overkill — just retain capacity.
                        let _ = list;
                    }
                    if s.awaiters.get(&seg).map(|l| l.is_empty()).unwrap_or(false) {
                        s.awaiters.remove(&seg);
                    }
                }
            });
            Err(SegmentWaitError::TimedOut(seg))
        }
    }
}

/// Prepare entry point for the `prepare_transcode` command: ensure session
/// exists and block (bounded) until segment 0 is ready.
pub async fn prepare_transcoded_stream(
    state: &ProxyState,
    input_url: &str,
    target_url: &str,
    referer: Option<&str>,
    variant: Option<&str>,
) -> Result<(), String> {
    if state.ffmpeg_path.is_none() {
        return Err("ffmpeg binary unavailable".to_string());
    }
    let key = ensure_hls_session(state, input_url, target_url, referer, variant).await?;
    let already = state.with_inner(|i| {
        i.sessions
            .get(&key)
            .map(|s| s.segments_available.contains(&0))
            .unwrap_or(false)
    });
    if already {
        update_session_progress(state, &key, "Ready to play");
        return Ok(());
    }
    state.set_progress(
        &key,
        TranscodeProgress {
            state: "running",
            progress_percent: Some(0.0),
            message: "Preparing first video segment...".to_string(),
        },
    );
    match tokio::time::timeout(
        Duration::from_millis(FIRST_SEGMENT_WAIT_MS),
        ensure_segment_ready(state, &key, 0),
    )
    .await
    {
        Ok(Ok(_)) => {
            update_session_progress(state, &key, "Ready to play");
            Ok(())
        }
        Ok(Err(e)) => {
            let msg = format!("First video segment not ready: {e}");
            state.set_progress(
                &key,
                TranscodeProgress {
                    state: "error",
                    progress_percent: None,
                    message: msg.clone(),
                },
            );
            Err(msg)
        }
        Err(_) => {
            let msg = format!(
                "Timed out after {}s waiting for first video segment (transcode still running in background)",
                FIRST_SEGMENT_WAIT_MS / 1000
            );
            state.set_progress(
                &key,
                TranscodeProgress {
                    state: "error",
                    progress_percent: None,
                    message: msg.clone(),
                },
            );
            Err(msg)
        }
    }
}
