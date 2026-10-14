//! AniList media queries — port of `anilist-media-page.ts`,
//! `anilist-search.ts`, `anilist-show-details.ts`, and
//! `anilist-popular-season.ts`.
//!
//! Responses are mapped to plain JSON matching the frontend's
//! `AniListShowDetails` / `AniListMediaPageResult` shapes.

use serde_json::{json, Value};

use super::api::post_graphql;
use super::oauth::stored_token;
use super::state::AniListState;

pub const MEDIA_PAGE_QUERY: &str = "
  query (
    $page: Int = 1
    $id: Int
    $type: MediaType
    $isAdult: Boolean = false
    $search: String
    $format: [MediaFormat]
    $status: MediaStatus
    $countryOfOrigin: CountryCode
    $source: MediaSource
    $season: MediaSeason
    $seasonYear: Int
    $year: String
    $onList: Boolean
    $yearLesser: FuzzyDateInt
    $yearGreater: FuzzyDateInt
    $episodeLesser: Int
    $episodeGreater: Int
    $durationLesser: Int
    $durationGreater: Int
    $chapterLesser: Int
    $chapterGreater: Int
    $volumeLesser: Int
    $volumeGreater: Int
    $licensedBy: [Int]
    $isLicensed: Boolean
    $genres: [String]
    $excludedGenres: [String]
    $tags: [String]
    $excludedTags: [String]
    $minimumTagRank: Int
    $sort: [MediaSort] = [POPULARITY_DESC, SCORE_DESC]
  ) {
    Page(page: $page, perPage: 6) {
      pageInfo {
        total
        perPage
        currentPage
        lastPage
        hasNextPage
      }
      media(
        id: $id
        type: $type
        season: $season
        format_in: $format
        status: $status
        countryOfOrigin: $countryOfOrigin
        source: $source
        search: $search
        onList: $onList
        seasonYear: $seasonYear
        startDate_like: $year
        startDate_lesser: $yearLesser
        startDate_greater: $yearGreater
        episodes_lesser: $episodeLesser
        episodes_greater: $episodeGreater
        duration_lesser: $durationLesser
        duration_greater: $durationGreater
        chapters_lesser: $chapterLesser
        chapters_greater: $chapterGreater
        volumes_lesser: $volumeLesser
        volumes_greater: $volumeGreater
        licensedById_in: $licensedBy
        isLicensed: $isLicensed
        genre_in: $genres
        genre_not_in: $excludedGenres
        tag_in: $tags
        tag_not_in: $excludedTags
        minimumTagRank: $minimumTagRank
        sort: $sort
        isAdult: $isAdult
      ) {
        id
        title {
          userPreferred
        }
        coverImage {
          extraLarge
          large
          color
        }
        startDate {
          year
          month
          day
        }
        endDate {
          year
          month
          day
        }
        bannerImage
        season
        seasonYear
        description
        type
        format
        status(version: 2)
        episodes
        duration
        chapters
        volumes
        genres
        isAdult
        averageScore
        popularity
        nextAiringEpisode {
          airingAt
          timeUntilAiring
          episode
        }
        mediaListEntry {
          id
          status
        }
        studios(isMain: true) {
          edges {
            isMain
            node {
              id
              name
            }
          }
        }
      }
    }
  }
";

const MEDIA_QUERY: &str = "
  query Media($mediaId: Int) {
    Media(id: $mediaId) {
      streamingEpisodes {
        thumbnail
        title
      }
      bannerImage
      coverImage {
        extraLarge
        large
        medium
      }
      description
      title {
        english
        romaji
        native
      }
      duration
      episodes
      averageScore
      season
      seasonYear
      status
      isFavourite
      mediaListEntry {
        id
        status
        progress
        startedAt { year month day }
        completedAt { year month day }
      }
    }
  }
";

fn opt_str(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => json!(s),
        _ => Value::Null,
    }
}

fn opt_num(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Number(n)) => json!(n),
        _ => Value::Null,
    }
}

fn opt_bool(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Bool(b)) => json!(b),
        _ => Value::Null,
    }
}

/// Map one Page.media node to the frontend's AniListShowDetails shape.
/// 1:1 with `mapPageMediaToShowDetails`.
pub fn map_page_media(raw: &Value) -> Value {
    let studios: Vec<Value> = raw
        .get("studios")
        .and_then(|s| s.get("edges"))
        .and_then(|e| e.as_array())
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| {
                    let node = e.get("node")?;
                    let id = node.get("id")?.as_i64()?;
                    let name = node.get("name")?.as_str()?;
                    Some(json!({ "isMain": e.get("isMain"), "node": { "id": id, "name": name } }))
                })
                .collect()
        })
        .unwrap_or_default();

    let next_airing = raw.get("nextAiringEpisode").and_then(|n| {
        let (a, t, e) = (
            n.get("airingAt")?.as_i64()?,
            n.get("timeUntilAiring")?.as_i64()?,
            n.get("episode")?.as_i64()?,
        );
        Some(json!({ "airingAt": a, "timeUntilAiring": t, "episode": e }))
    });

    let mle = raw.get("mediaListEntry").and_then(|m| {
        let (id, status) = (m.get("id")?.as_i64()?, m.get("status")?.as_str()?);
        Some(json!({ "id": id, "status": status }))
    });

    let title = raw.get("title").and_then(|t| t.get("userPreferred")).and_then(|u| u.as_str());
    let cover = raw.get("coverImage");

    json!({
        "id": raw.get("id"),
        "title": title.map(|u| json!({ "userPreferred": u })).unwrap_or(Value::Null),
        "coverImage": cover.map(|c| json!({
            "extraLarge": opt_str(c.get("extraLarge")),
            "large": opt_str(c.get("large")),
            "medium": Value::Null,
            "color": opt_str(c.get("color")),
        })).unwrap_or(Value::Null),
        "startDate": raw.get("startDate").cloned().unwrap_or(Value::Null),
        "endDate": raw.get("endDate").cloned().unwrap_or(Value::Null),
        "bannerImage": opt_str(raw.get("bannerImage")),
        "season": opt_str(raw.get("season")),
        "seasonYear": opt_num(raw.get("seasonYear")),
        "description": opt_str(raw.get("description")),
        "type": opt_str(raw.get("type")),
        "format": opt_str(raw.get("format")),
        "status": opt_str(raw.get("status")),
        "episodes": opt_num(raw.get("episodes")),
        "duration": opt_num(raw.get("duration")),
        "chapters": opt_num(raw.get("chapters")),
        "volumes": opt_num(raw.get("volumes")),
        "genres": raw.get("genres").cloned().unwrap_or(Value::Null),
        "isAdult": opt_bool(raw.get("isAdult")),
        "averageScore": opt_num(raw.get("averageScore")),
        "popularity": opt_num(raw.get("popularity")),
        "nextAiringEpisode": next_airing.unwrap_or(Value::Null),
        "mediaListEntry": mle.unwrap_or(Value::Null),
        "studios": if studios.is_empty() { Value::Null } else { json!(studios) },
    })
}

/// Normalize `{ sort: "X" }` → `{ sort: ["X"] }`, drop undefined-equivalents.
/// 1:1 with `graphqlVariablesFromInput` (JSON nulls survive; upstream only
/// dropped `undefined`, which has no JSON equivalent).
pub fn normalize_page_variables(input: &Value) -> Value {
    let mut vars = match input.as_object() {
        Some(map) => map.clone(),
        None => return json!({}),
    };
    if let Some(sort) = vars.remove("sort") {
        match sort {
            Value::String(s) => {
                vars.insert("sort".to_string(), json!([s]));
            }
            Value::Null => {}
            other => {
                vars.insert("sort".to_string(), other);
            }
        }
    }
    Value::Object(vars)
}

pub async fn fetch_media_page(
    state: &AniListState,
    variables: Value,
) -> Result<Value, String> {
    let data = post_graphql(
        &state.http,
        MEDIA_PAGE_QUERY,
        normalize_page_variables(&variables),
        None,
    )
    .await?;
    let page = data.get("Page").ok_or_else(|| "AniList page response incomplete".to_string())?;
    let page_info = page.get("pageInfo").ok_or_else(|| "AniList page response incomplete".to_string())?;
    let media: Vec<Value> = page
        .get("media")
        .and_then(|m| m.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|m| !m.is_null())
                .map(map_page_media)
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "pageInfo": page_info, "media": media }))
}

pub async fn show_details(
    state: &AniListState,
    app: &tauri::AppHandle,
    media_id: i64,
) -> Result<Value, String> {
    if media_id <= 0 {
        return Err("Invalid AniList media id".to_string());
    }
    let token = stored_token(app);
    let data = post_graphql(
        &state.http,
        MEDIA_QUERY,
        json!({ "mediaId": media_id }),
        token.as_deref(),
    )
    .await?;
    data.get("Media")
        .filter(|m| !m.is_null())
        .cloned()
        .ok_or_else(|| "AniList media not found".to_string())
}

// ---------------------------------------------------------------------------
// seasons (no chrono: civil date from unix time via Hinnant's algorithm)
// ---------------------------------------------------------------------------

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn now_ymd() -> (i64, u32, u32) {
    civil_from_days(unix_days())
}

fn unix_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86400)
        .unwrap_or(0) as i64
}

/// Today's civil date, shared with the list-sync `startedAt`/`completedAt`.
pub(crate) fn today_ymd() -> (i64, u32, u32) {
    civil_from_days(unix_days())
}

/// AniList season year for winter is the calendar year containing January.
pub fn current_anime_season_at(year: i64, month: u32) -> (String, i64) {
    match month {
        12 => ("WINTER".to_string(), year + 1),
        1 | 2 => ("WINTER".to_string(), year),
        3..=5 => ("SPRING".to_string(), year),
        6..=8 => ("SUMMER".to_string(), year),
        _ => ("FALL".to_string(), year),
    }
}

pub async fn popular_season(state: &AniListState) -> Result<Value, String> {
    let (year, month, _) = now_ymd();
    let (season, season_year) = current_anime_season_at(year, month);
    let season_key = format!("{season}-{season_year}");

    if let Some(cached) = state.season_cache.lock().unwrap().as_ref() {
        if cached.season_key == season_key
            && cached.fetched_at.elapsed() < std::time::Duration::from_secs(3600)
        {
            return Ok(cached.data.clone());
        }
    }

    let result = fetch_media_page(
        state,
        json!({
            "page": 1,
            "type": "ANIME",
            "seasonYear": season_year,
            "season": season,
            "sort": ["POPULARITY_DESC"],
        }),
    )
    .await?;
    let media = result.get("media").cloned().unwrap_or(json!([]));
    *state.season_cache.lock().unwrap() = Some(crate::anilist::state::SeasonCache {
        season_key,
        fetched_at: std::time::Instant::now(),
        data: media.clone(),
    });
    Ok(media)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn season_boundaries() {
        assert_eq!(current_anime_season_at(2026, 12), ("WINTER".to_string(), 2027));
        assert_eq!(current_anime_season_at(2026, 1), ("WINTER".to_string(), 2026));
        assert_eq!(current_anime_season_at(2026, 2), ("WINTER".to_string(), 2026));
        assert_eq!(current_anime_season_at(2026, 4), ("SPRING".to_string(), 2026));
        assert_eq!(current_anime_season_at(2026, 7), ("SUMMER".to_string(), 2026));
        assert_eq!(current_anime_season_at(2026, 10), ("FALL".to_string(), 2026));
    }

    #[test]
    fn civil_date_sanity() {
        // Cross-checked with Python datetime (shell `date` truncates across DST).
        assert_eq!(civil_from_days(20739), (2026, 10, 13));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(18628), (2021, 1, 1));
    }

    #[test]
    fn sort_string_normalized_to_array() {
        let vars = normalize_page_variables(&json!({ "search": "naruto", "sort": "POPULARITY_DESC" }));
        assert_eq!(vars.get("sort"), Some(&json!(["POPULARITY_DESC"])));
        let vars = normalize_page_variables(&json!({ "sort": ["A", "B"] }));
        assert_eq!(vars.get("sort"), Some(&json!(["A", "B"])));
    }

    #[test]
    fn page_media_mapping() {
        let raw = json!({
            "id": 1,
            "title": { "userPreferred": "Cowboy Bebop" },
            "coverImage": { "extraLarge": "x", "large": "y", "color": "#fff" },
            "nextAiringEpisode": null,
            "mediaListEntry": { "id": 5, "status": "CURRENT" },
            "studios": { "edges": [
                { "isMain": true, "node": { "id": 14, "name": "Sunrise" } },
                { "isMain": false, "node": { "id": null, "name": "x" } }
            ] }
        });
        let mapped = map_page_media(&raw);
        assert_eq!(mapped["title"]["userPreferred"], json!("Cowboy Bebop"));
        assert_eq!(mapped["coverImage"]["medium"], Value::Null);
        assert_eq!(mapped["mediaListEntry"]["id"], json!(5));
        assert_eq!(mapped["studios"].as_array().unwrap().len(), 1);
        assert_eq!(mapped["nextAiringEpisode"], Value::Null);
    }

    #[tokio::test]
    async fn live_media_search() {
        let state = AniListState::new();
        let result = fetch_media_page(&state, json!({ "search": "Cowboy Bebop", "type": "ANIME" }))
            .await
            .expect("live search");
        let media = result["media"].as_array().expect("media array");
        assert!(!media.is_empty());
        assert!(media.iter().any(|m| m["id"] == json!(1)));
    }

    #[tokio::test]
    async fn live_show_details() {
        let state = AniListState::new();
        // Token-less AppHandle isn't available in unit tests; call the
        // underlying query directly without a token.
        let data = post_graphql(
            &state.http,
            MEDIA_QUERY,
            json!({ "mediaId": 1 }),
            None,
        )
        .await
        .expect("live details");
        let media = &data["Media"];
        assert_eq!(media["title"]["english"], json!("Cowboy Bebop"));
    }
}
