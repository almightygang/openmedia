//! AniList favourites — port of `anilist-favourites.ts`.

use serde_json::{json, Value};

use super::api::post_authed_graphql;
use super::state::AniListState;

const FAVOURITES_QUERY: &str = "
  query ($page: Int, $perPage: Int) {
    Viewer {
      favourites {
        anime(page: $page, perPage: $perPage) {
          pageInfo {
            total
            perPage
            currentPage
            lastPage
            hasNextPage
          }
          nodes {
            id
            title { english romaji native userPreferred }
            coverImage { extraLarge large }
            bannerImage
            episodes
            averageScore
            season
            seasonYear
            status
            isFavourite
          }
        }
      }
    }
  }
";

const TOGGLE_FAVOURITE_MUTATION: &str = "
  mutation ($animeId: Int) {
    ToggleFavourite(animeId: $animeId) {
      anime {
        nodes {
          id
        }
      }
    }
  }
";

/// 1:1 with `mapFavourite` (unlisted ids are dropped; default favourite).
pub fn map_favourite(raw: &Value) -> Option<Value> {
    let id = raw.get("id")?.as_i64()?;
    if id <= 0 {
        return None;
    }
    let is_favourite = raw.get("isFavourite").and_then(|v| v.as_bool()).unwrap_or(true);
    Some(json!({
        "id": id,
        "title": raw.get("title").cloned().unwrap_or(Value::Null),
        "coverImage": raw.get("coverImage").cloned().unwrap_or(Value::Null),
        "bannerImage": raw.get("bannerImage").cloned().unwrap_or(Value::Null),
        "episodes": raw.get("episodes").cloned().unwrap_or(Value::Null),
        "averageScore": raw.get("averageScore").cloned().unwrap_or(Value::Null),
        "season": raw.get("season").cloned().unwrap_or(Value::Null),
        "seasonYear": raw.get("seasonYear").cloned().unwrap_or(Value::Null),
        "status": raw.get("status").cloned().unwrap_or(Value::Null),
        "isFavourite": is_favourite,
    }))
}

pub async fn fetch_favourites(
    state: &AniListState,
    app: &tauri::AppHandle,
    page: i64,
) -> Result<Value, String> {
    let data = post_authed_graphql(
        state,
        app,
        FAVOURITES_QUERY,
        json!({ "page": page, "perPage": 50 }),
    )
    .await?;
    let anime = data
        .get("Viewer")
        .and_then(|v| v.get("favourites"))
        .and_then(|f| f.get("anime"))
        .unwrap_or(&Value::Null);
    let page_info = anime.get("pageInfo").cloned().unwrap_or(json!({}));
    let media: Vec<Value> = anime
        .get("nodes")
        .and_then(|n| n.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter(|n| !n.is_null())
                .filter_map(map_favourite)
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "media": media, "pageInfo": page_info }))
}

pub async fn toggle_favourite(
    state: &AniListState,
    app: &tauri::AppHandle,
    media_id: i64,
) -> Result<bool, String> {
    if media_id <= 0 {
        return Err("Invalid AniList media id.".to_string());
    }
    let data = post_authed_graphql(
        state,
        app,
        TOGGLE_FAVOURITE_MUTATION,
        json!({ "animeId": media_id }),
    )
    .await?;
    let on_list = data
        .get("ToggleFavourite")
        .and_then(|t| t.get("anime"))
        .and_then(|a| a.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .any(|node| node.get("id").and_then(|id| id.as_i64()) == Some(media_id))
        })
        .unwrap_or(false);
    Ok(on_list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favourite_mapping() {
        let ok = map_favourite(&json!({ "id": 1, "title": null })).unwrap();
        assert_eq!(ok["isFavourite"], json!(true));
        assert!(map_favourite(&json!({ "id": 0 })).is_none());
        assert!(map_favourite(&json!({})).is_none());
    }
}
