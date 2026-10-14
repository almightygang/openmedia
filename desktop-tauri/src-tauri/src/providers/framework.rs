//! Shared provider plumbing: HTTP context, fetch helpers, Cloudflare
//! detection, bounded-concurrency mapping.

use std::time::Duration;

use serde_json::Value;

use crate::proxy::state::CHROME_UA;

pub const FETCH_TIMEOUT_SECS: u64 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    Sub,
    Dub,
}

impl StreamMode {
    pub fn from_str(s: Option<&str>) -> Self {
        match s {
            Some("dub") => Self::Dub,
            _ => Self::Sub,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sub => "sub",
            Self::Dub => "dub",
        }
    }
}

/// Shared outbound client for all providers.
#[derive(Clone)]
pub struct ProviderContext {
    pub http: reqwest::Client,
}

impl ProviderContext {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .cookie_store(true)
                .user_agent(CHROME_UA)
                .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
                .build()
                .expect("provider reqwest client"),
        }
    }
}

/// True when the body is a Cloudflare / DDoS-guard challenge page.
pub fn looks_like_cf_block(text: &str) -> bool {
    let head: String = text.chars().take(800).collect::<String>().to_lowercase();
    head.contains("just a moment")
        || head.contains("attention required")
        || head.contains("checking your browser")
        || head.contains("cf-error-details")
        || head.contains("sorry, you have been blocked")
}

pub fn cf_blocked_error(site: &str) -> String {
    format!("{site} is behind a Cloudflare challenge this build cannot clear yet (no hidden browser)")
}

/// GET text with UA + referer. CF challenge bodies become errors.
pub async fn fetch_text(
    ctx: &ProviderContext,
    site: &str,
    url: &str,
    referer: Option<&str>,
    accept: Option<&str>,
) -> Result<String, String> {
    let mut req = ctx.http.get(url);
    if let Some(r) = referer {
        req = req.header("Referer", r);
    }
    req = req.header("Accept", accept.unwrap_or("*/*"));
    req = req.header("Accept-Language", "en-US,en;q=0.9");
    let res = req.send().await.map_err(|e| format!("{site} request failed: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("{site} request failed ({status}): {url}"));
    }
    let text = res.text().await.map_err(|e| format!("{site} read failed: {e}"))?;
    if looks_like_cf_block(&text) {
        return Err(cf_blocked_error(site));
    }
    Ok(text)
}

/// GET JSON with UA + referer.
pub async fn fetch_json(
    ctx: &ProviderContext,
    site: &str,
    url: &str,
    referer: Option<&str>,
) -> Result<Value, String> {
    let text = fetch_text(
        ctx,
        site,
        url,
        referer,
        Some("application/json, text/plain, */*"),
    )
    .await?;
    serde_json::from_str(&text).map_err(|_| format!("{site} returned invalid JSON for {url}"))
}

pub fn as_str(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn as_num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Compiled-regex cache: identical semantics to `Regex::new(pat).unwrap()`
/// without recompiling on hot parse paths. Patterns here are all literals
/// (or bounded dynamic strings); invalid patterns panic like `unwrap`.
pub fn cached_regex(pattern: &str) -> &'static regex::Regex {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, &'static regex::Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(re) = cache.lock().unwrap().get(pattern) {
        return re;
    }
    let leaked: &'static regex::Regex =
        Box::leak(Box::new(regex::Regex::new(pattern).expect("valid regex")));
    cache.lock().unwrap().insert(pattern.to_string(), leaked);
    leaked
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Live-test helper: transport blocks (CF/WAF/rate-limit/outage) skip
    /// with a notice; contract violations (200 + wrong shape) still fail.
    pub fn live_or_skip(result: Result<Value, String>, label: &str) -> Option<Value> {
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                let transient = ["Cloudflare", "403", "429", "timed out", "error sending request", "Try again", "502", "503", "500"]
                    .iter()
                    .any(|m| e.contains(m));
                if transient {
                    eprintln!("SKIP {label} (site blocked/unreachable): {e}");
                    None
                } else {
                    panic!("{label}: {e}");
                }
            }
        }
    }

    #[test]
    fn cf_detection() {
        assert!(looks_like_cf_block("<title>Just a moment...</title>"));
        assert!(!looks_like_cf_block("<html><body>anime list</body></html>"));
    }

}
