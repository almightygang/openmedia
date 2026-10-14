//! AnimeParadise provider — port of `animeparadise-stream-provider.ts`.
//!
//! Pure REST (`api.animeparadise.moe`), no scraping. Sub-only.

use serde_json::{json, Value};

use super::framework::{as_num, as_str, cached_regex, fetch_json, ProviderContext, StreamMode};
use super::TtlCache;

const API_BASE: &str = "https://api.animeparadise.moe";
const STREAM_BASE: &str = "https://stream.animeparadise.moe";
const REFERER: &str = "https://animeparadise.moe/";
const SEARCH_LIMIT: usize = 36;

pub struct AnimeParadiseProvider {
    ctx: ProviderContext,
    episodes: TtlCache,
}

impl AnimeParadiseProvider {
    pub fn new(ctx: ProviderContext) -> Self {
        Self { ctx, episodes: TtlCache::new(300) }
    }

    async fn api_get(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let mut url = format!("{API_BASE}{path}");
        if !params.is_empty() {
            let q: Vec<String> = params
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{k}={}",
                        url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
                    )
                })
                .collect();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        // apiGet sends no explicit Referer; default UA/Accept suffice.
        fetch_json(&self.ctx, "AnimeParadise", &url, None).await
    }

    fn map_title(item: &Value, fallback: &str) -> Value {
        let alt = item.get("alternativeTitle");
        json!({
            "english": alt.and_then(|a| as_str(a.get("english")))
                .or_else(|| as_str(item.get("title")))
                .unwrap_or_else(|| fallback.to_string()),
            "romanji": alt.and_then(|a| as_str(a.get("romaji"))),
            "native": alt.and_then(|a| as_str(a.get("native"))),
        })
    }

    fn thumbnail(item: &Value, episode_image: Option<&Value>) -> Value {
        if let Some(img) = as_str(episode_image) {
            return json!(img);
        }
        let poster = item.get("posterImage");
        for key in ["medium", "large", "small"] {
            if let Some(u) = poster.and_then(|p| as_str(p.get(key))) {
                return json!(u);
            }
        }
        Value::Null
    }

    fn anilist_id(item: &Value) -> Option<String> {
        if let Some(n) = item.get("mappings").and_then(|m| m.get("anilist")).and_then(|v| v.as_i64()) {
            if n > 0 {
                return Some(n.to_string());
            }
        }
        // AniList CDN poster ids look like /bx12345-.
        let poster = item
            .get("posterImage")
            .and_then(|p| as_str(p.get("large")))
            .or_else(|| item.get("posterImage").and_then(|p| as_str(p.get("medium"))))
            .or_else(|| item.get("posterImage").and_then(|p| as_str(p.get("small"))))
            .unwrap_or_default();
        let re = cached_regex(r"/bx(\d+)-");
        re.captures(&poster).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
    }

    fn season(item: &Value) -> Value {
        let quarter = item.get("animeSeason").and_then(|s| as_str(s.get("season")));
        let mut year = item
            .get("animeSeason")
            .and_then(|s| s.get("year"))
            .and_then(|v| v.as_i64())
            .or_else(|| item.get("year").and_then(|v| v.as_i64()));
        if year.is_none() {
            if let Some(released) = as_str(item.get("released")) {
                // "2024-01-01" → leading year.
                if let Ok(y) = released[..4.min(released.len())].parse::<i64>() {
                    year = Some(y);
                }
            }
        }
        if quarter.is_none() && year.is_none() {
            return Value::Null;
        }
        json!({
            "quarter": quarter,
            "year": year.filter(|y| *y > 0),
        })
    }

    fn map_search_item(&self, item: &Value) -> Option<Value> {
        let provider_id = as_str(item.get("_id"))?;
        let anilist = Self::anilist_id(item).unwrap_or_else(|| provider_id.clone());
        let ep_count = as_num(item.get("episodes").or_else(|| item.get("episodeCount")))
            .map(|n| n as i64);
        let score = as_num(item.get("rate"));
        Some(json!({
            "id": anilist,
            "providerId": provider_id,
            "title": Self::map_title(item, &provider_id),
            "thumbnail": Self::thumbnail(item, None),
            "availableEpisodes": { "sub": ep_count },
            "score": score,
            "type": "TV",
            "season": Self::season(item),
        }))
    }

    async fn load_episodes(&self, provider_id: &str) -> Result<Vec<Value>, String> {
        if let Some(cached) = self.episodes.get(provider_id) {
            return Ok(cached.as_array().cloned().unwrap_or_default());
        }
        let json = self
            .api_get(&format!("/anime/{provider_id}/episode"), &[])
            .await?;
        let rows: Vec<Value> = json
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        self.episodes.set(provider_id, json!(rows));
        Ok(rows)
    }

    async fn fetch_episodes(&self, provider_id: &str) -> Result<Vec<Value>, String> {
        self.load_episodes(provider_id).await
    }

    async fn episode_details(&self, provider_id: &str, uid: &str) -> Result<Value, String> {
        let json = self
            .api_get(&format!("/ep/{uid}"), &[("origin", provider_id)])
            .await?;
        Ok(json.get("data").cloned().unwrap_or(json!({})))
    }

    pub async fn search(&self, query: &str) -> Result<Value, String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            let episodes = self.get_recent_uploads(1, SEARCH_LIMIT, StreamMode::Sub).await?;
            let results: Vec<Value> = episodes
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|ep| {
                    let index = ep.get("index").and_then(|v| v.as_i64()).unwrap_or(0);
                    json!({
                        "id": ep.get("id"),
                        "providerId": ep.get("providerId"),
                        "title": {
                            "english": ep.get("title").and_then(|t| as_str(t.get("english"))).or_else(|| as_str(ep.get("providerId"))),
                            "romanji": ep.get("title").and_then(|t| as_str(t.get("romanji"))),
                            "native": ep.get("title").and_then(|t| as_str(t.get("native"))),
                        },
                        "thumbnail": ep.get("thumbnail"),
                        "availableEpisodes": { "sub": if index > 0 { Some(index) } else { None } },
                    })
                })
                .collect();
            return Ok(json!(results));
        }
        let json = self
            .api_get("/search", &[("q", trimmed), ("limit", &SEARCH_LIMIT.to_string())])
            .await?;
        let rows: &[Value] = json.get("data").and_then(|d| d.as_array()).map(|v| v.as_slice()).unwrap_or(&[]);
        let results: Vec<Value> = rows.iter().filter_map(|item| self.map_search_item(item)).collect();
        Ok(json!(results))
    }

    pub async fn get_recent_uploads(
        &self,
        page: i64,
        limit: usize,
        mode: StreamMode,
    ) -> Result<Value, String> {
        if mode == StreamMode::Dub {
            return Ok(json!([]));
        }
        let json = self
            .api_get(
                "/search",
                &[("limit", &limit.max(1).to_string()), ("page", &page.max(1).to_string())],
            )
            .await?;
        let rows: &[Value] = json.get("data").and_then(|d| d.as_array()).map(|v| v.as_slice()).unwrap_or(&[]);
        let episodes: Vec<Value> = rows
            .iter()
            .map(|item| {
                let provider_id = as_str(item.get("_id")).unwrap_or_default();
                let anilist = Self::anilist_id(item).unwrap_or_else(|| provider_id.clone());
                let index = as_num(item.get("episodes").or_else(|| item.get("episodeCount")))
                    .map(|n| n as i64)
                    .unwrap_or(0);
                json!({
                    "id": anilist,
                    "providerId": provider_id,
                    "title": Self::map_title(item, &provider_id),
                    "thumbnail": Self::thumbnail(item, None),
                    "index": index,
                    "mode": mode.as_str(),
                })
            })
            .collect();
        Ok(json!(episodes))
    }

    pub async fn get_episodes_list(&self, provider_id: &str) -> Result<Value, String> {
        let rows = self.fetch_episodes(provider_id).await?;
        let mut numbers: Vec<i64> = rows
            .iter()
            .filter_map(|item| as_num(item.get("number")).map(|n| n as i64))
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        Ok(json!(numbers.iter().map(|n| n.to_string()).collect::<Vec<_>>()))
    }

    pub async fn get_show_details(&self, provider_id: &str) -> Result<Value, String> {
        let rows = self.fetch_episodes(provider_id).await?;
        let first_uid = rows.first().and_then(|e| as_str(e.get("uid"))).ok_or_else(|| {
            format!("AnimeParadise show not found: {provider_id}")
        })?;
        let details = self.episode_details(provider_id, &first_uid).await?;
        let anime_data = details.get("animeData");
        let link = anime_data.and_then(|a| as_str(a.get("link")));
        let fallback_title = anime_data
            .and_then(|a| as_str(a.get("title")))
            .unwrap_or_else(|| provider_id.to_string());

        let mut fetched: Option<Value> = None;
        if let Some(link) = link {
            if let Ok(json) = self.api_get(&format!("/anime/{link}"), &[]).await {
                fetched = json.get("data").cloned();
            }
        }
        let merged = fetched.as_ref().or(anime_data);
        let name = fetched
            .as_ref()
            .and_then(|d| as_str(d.get("title")))
            .unwrap_or(fallback_title);
        let thumb = fetched
            .as_ref()
            .and_then(|d| d.get("posterImage").cloned())
            .map(|p| Self::thumbnail(&json!({ "posterImage": p }), rows.first().and_then(|e| e.get("image"))))
            .unwrap_or_else(|| {
                Self::thumbnail(
                    &json!({}),
                    rows.first().and_then(|e| e.get("image")),
                )
            });
        let id = merged
            .and_then(Self::anilist_id)
            .unwrap_or_else(|| provider_id.to_string());
        let typ = fetched
            .as_ref()
            .and_then(|d| as_str(d.get("type")))
            .unwrap_or_else(|| "TV".to_string());
        let description = fetched
            .as_ref()
            .and_then(|d| as_str(d.get("synopsys")));
        Ok(json!({
            "id": id,
            "providerId": provider_id,
            "name": name,
            "thumbnail": thumb,
            "type": typ,
            "description": description,
        }))
    }

    pub async fn get_stream_url(
        &self,
        provider_id: Option<&str>,
        episode: &str,
        mode: StreamMode,
    ) -> Result<Value, String> {
        if mode == StreamMode::Dub {
            return Err("AnimeParadise does not provide dubbed streams".to_string());
        }
        let provider_id =
            provider_id.ok_or_else(|| "Missing providerId for AnimeParadise stream lookup".to_string())?;
        let episode_number: f64 = episode
            .parse()
            .map_err(|_| format!("Invalid episode number for AnimeParadise: {episode}"))?;
        if episode_number < 1.0 {
            return Err(format!("Invalid episode number for AnimeParadise: {episode}"));
        }
        let rows = self.fetch_episodes(provider_id).await?;
        let matched = rows.iter().find(|item| {
            as_num(item.get("number")).map(|n| (n - episode_number).abs() < 1e-9).unwrap_or(false)
        });
        let uid = matched.and_then(|m| as_str(m.get("uid"))).ok_or_else(|| {
            format!("AnimeParadise episode {episode} not found for {provider_id}")
        })?;
        let details = self.episode_details(provider_id, &uid).await?;
        let stream_link = details
            .get("episode")
            .and_then(|e| as_str(e.get("streamLink")))
            .ok_or_else(|| "AnimeParadise: no streamLink in response".to_string())?;
        let url = format!(
            "{STREAM_BASE}/m3u8?url={}",
            url::form_urlencoded::byte_serialize(stream_link.as_bytes()).collect::<String>()
        );
        Ok(json!({ "url": url, "referer": REFERER }))
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::providers::framework::tests::live_or_skip;

    #[tokio::test]
    async fn live_search() {
        let p = AnimeParadiseProvider::new(ProviderContext::new());
        if let Some(shows) = live_or_skip(p.search("naruto").await, "animeparadise search") {
            assert!(!shows.as_array().expect("array").is_empty());
        }
    }

    #[tokio::test]
    async fn live_end_to_end() {
        let p = AnimeParadiseProvider::new(ProviderContext::new());
        let Some(shows) = live_or_skip(p.search("naruto").await, "paradise e2e search") else { return };
        let first = shows.as_array().expect("array").first().cloned().expect("hit");
        let provider_id = first["providerId"].as_str().expect("providerId").to_string();

        let Some(details) = live_or_skip(p.get_show_details(&provider_id).await, "paradise e2e details") else { return };
        assert!(!details["name"].as_str().unwrap_or_default().is_empty());

        let Some(episodes) = live_or_skip(p.get_episodes_list(&provider_id).await, "paradise e2e episodes") else { return };
        let ep = episodes.as_array().expect("array").first().and_then(|v| v.as_str()).expect("ep").to_string();

        // Stream URL shape only — the URL itself may be geo-fenced.
        if let Some(stream) = live_or_skip(p.get_stream_url(Some(&provider_id), &ep, StreamMode::Sub).await, "paradise e2e stream") {
            assert!(stream["url"].as_str().map(|u| u.starts_with("http")).unwrap_or(false));
            assert!(stream.get("referer").and_then(|v| v.as_str()).is_some());
        }
    }
}
