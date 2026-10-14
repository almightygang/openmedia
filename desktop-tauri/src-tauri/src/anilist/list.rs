//! AniList list entries — port of `anilist-list.ts`.
//!
//! The save-input computation is factored pure (`compute_save_input`) so
//! the status-transition matrix is unit-testable without network.

use serde_json::{json, Value};

use super::api::{post_authed_graphql, viewer_id};
use super::state::AniListState;

const MEDIA_LIST_FIELDS: &str = "
  id
  status
  progress
  score
  repeat
  startedAt { year month day }
  completedAt { year month day }
  media {
    id
    title { english romaji native userPreferred }
    coverImage { extraLarge large }
    bannerImage
    episodes
    averageScore
    season
    seasonYear
    status
  }
";

const MEDIA_LIST_PAGE_QUERY: &str = "
  query ($userId: Int, $status: MediaListStatus, $page: Int, $perPage: Int) {
    Page(page: $page, perPage: $perPage) {
      pageInfo {
        total
        perPage
        currentPage
        lastPage
        hasNextPage
      }
      mediaList(userId: $userId, status: $status, sort: UPDATED_TIME_DESC, type: ANIME) {
        __FIELDS__
      }
    }
  }
";

const SAVE_MEDIA_LIST_ENTRY_MUTATION: &str = "
  mutation (
    $id: Int
    $mediaId: Int
    $status: MediaListStatus
    $progress: Int
    $startedAt: FuzzyDateInput
    $completedAt: FuzzyDateInput
  ) {
    SaveMediaListEntry(
      id: $id
      mediaId: $mediaId
      status: $status
      progress: $progress
      startedAt: $startedAt
      completedAt: $completedAt
    ) {
      __FIELDS__
    }
  }
";

const DELETE_MEDIA_LIST_ENTRY_MUTATION: &str = "
  mutation ($id: Int) {
    DeleteMediaListEntry(id: $id) {
      deleted
    }
  }
";

fn with_fields(template: &str) -> String {
    template.replace("__FIELDS__", MEDIA_LIST_FIELDS)
}

fn positive_i64(v: Option<&Value>) -> Option<i64> {
    v.and_then(|x| x.as_i64()).filter(|n| *n > 0)
}

/// Map one raw list entry; None when media/status are unusable.
/// 1:1 with `mapListEntry`.
pub fn map_list_entry(raw: &Value) -> Option<Value> {
    let media_id = positive_i64(raw.get("media")?.get("id"))?;
    let status = raw.get("status")?.as_str()?;
    let media = raw.get("media")?;
    Some(json!({
        "id": raw.get("id"),
        "status": status,
        "progress": raw.get("progress").and_then(|p| p.as_i64()).unwrap_or(0),
        "score": raw.get("score"),
        "repeat": raw.get("repeat").and_then(|p| p.as_i64()).unwrap_or(0),
        "startedAt": raw.get("startedAt").cloned().unwrap_or(Value::Null),
        "completedAt": raw.get("completedAt").cloned().unwrap_or(Value::Null),
        "media": {
            "id": media_id,
            "title": media.get("title").cloned().unwrap_or(Value::Null),
            "coverImage": media.get("coverImage").cloned().unwrap_or(Value::Null),
            "bannerImage": media.get("bannerImage").cloned().unwrap_or(Value::Null),
            "episodes": media.get("episodes").cloned().unwrap_or(Value::Null),
            "averageScore": media.get("averageScore").cloned().unwrap_or(Value::Null),
            "season": media.get("season").cloned().unwrap_or(Value::Null),
            "seasonYear": media.get("seasonYear").cloned().unwrap_or(Value::Null),
            "status": media.get("status").cloned().unwrap_or(Value::Null),
        },
    }))
}

fn today_fuzzy_date() -> Value {
    let (y, m, d) = super::media::today_ymd();
    json!({ "year": y, "month": m, "day": d })
}

pub async fn fetch_media_list(
    state: &AniListState,
    app: &tauri::AppHandle,
    status: &str,
    page: i64,
) -> Result<Value, String> {
    let user_id = viewer_id(state, app).await?;
    let data = post_authed_graphql(
        state,
        app,
        &with_fields(MEDIA_LIST_PAGE_QUERY),
        json!({ "userId": user_id, "status": status, "page": page, "perPage": 50 }),
    )
    .await?;
    let page_obj = data.get("Page").unwrap_or(&Value::Null);
    let page_info = page_obj.get("pageInfo").cloned().unwrap_or(json!({}));
    let entries: Vec<Value> = page_obj
        .get("mediaList")
        .and_then(|l| l.as_array())
        .map(|items| items.iter().filter_map(|e| (!e.is_null()).then(|| map_list_entry(e)).flatten()).collect())
        .unwrap_or_default();
    Ok(json!({ "entries": entries, "pageInfo": page_info }))
}

pub async fn save_entry(
    state: &AniListState,
    app: &tauri::AppHandle,
    input: &Value,
) -> Result<Value, String> {
    let mut variables = json!({});
    for key in ["id", "mediaId", "status", "progress", "startedAt", "completedAt"] {
        if let Some(v) = input.get(key) {
            if !v.is_null() {
                variables[key] = v.clone();
            }
        }
    }
    let data = post_authed_graphql(
        state,
        app,
        &with_fields(SAVE_MEDIA_LIST_ENTRY_MUTATION),
        variables,
    )
    .await?;
    data.get("SaveMediaListEntry")
        .and_then(map_list_entry)
        .ok_or_else(|| "Failed to save AniList list entry.".to_string())
}

pub async fn delete_entry(
    state: &AniListState,
    app: &tauri::AppHandle,
    list_entry_id: i64,
) -> Result<(), String> {
    post_authed_graphql(
        state,
        app,
        DELETE_MEDIA_LIST_ENTRY_MUTATION,
        json!({ "id": list_entry_id }),
    )
    .await?;
    Ok(())
}

/// Pure status-transition computation for watch-progress sync.
/// 1:1 with `syncAniListWatchProgress`'s decision logic (minus the network).
pub fn compute_save_input(
    media_id: i64,
    episode_number: i64,
    total_episodes: Option<i64>,
    current_status: Option<&str>,
    list_entry_id: Option<i64>,
    episode_completed: bool,
) -> Option<Value> {
    if media_id <= 0 || episode_number <= 0 {
        return None;
    }
    let total = total_episodes.filter(|t| *t > 0);
    let is_last = total.map(|t| episode_number >= t).unwrap_or(false);

    let status = if is_last && episode_completed {
        "COMPLETED"
    } else if current_status == Some("COMPLETED") || current_status == Some("REPEATING") {
        "REPEATING"
    } else {
        "CURRENT"
    };

    let mut input = json!({ "status": status, "progress": episode_number });
    if let Some(id) = list_entry_id {
        input["id"] = json!(id);
    } else {
        input["mediaId"] = json!(media_id);
    }

    let is_new = matches!(current_status, None | Some("PLANNING") | Some("PAUSED"));
    if (status == "CURRENT" || status == "REPEATING") && is_new {
        input["startedAt"] = today_fuzzy_date();
    }
    if status == "COMPLETED" {
        input["completedAt"] = today_fuzzy_date();
        if let Some(t) = total {
            input["progress"] = json!(t);
        }
    }
    Some(input)
}

pub async fn sync_watch_progress(
    state: &AniListState,
    app: &tauri::AppHandle,
    input: &Value,
) -> Result<Value, String> {
    let media_id = input.get("mediaId").and_then(|v| v.as_i64()).unwrap_or(0);
    let episode_number = input.get("episodeNumber").and_then(|v| v.as_i64()).unwrap_or(0);
    let total = input.get("totalEpisodes").and_then(|v| v.as_i64());
    let current_status = input.get("currentStatus").and_then(|v| v.as_str());
    let list_entry_id = input.get("listEntryId").and_then(|v| v.as_i64());
    let completed = input.get("episodeCompleted").and_then(|v| v.as_bool()).unwrap_or(false);

    let Some(save_input) = compute_save_input(
        media_id,
        episode_number,
        total,
        current_status,
        list_entry_id,
        completed,
    ) else {
        return Ok(Value::Null);
    };
    let entry = save_entry(state, app, &save_input).await?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_mapping_valid() {
        let raw = json!({
            "id": 7, "status": "CURRENT", "progress": 3, "score": 8, "repeat": 0,
            "startedAt": null, "completedAt": null,
            "media": { "id": 1, "title": { "english": "Cowboy Bebop" } }
        });
        let mapped = map_list_entry(&raw).expect("mapped");
        assert_eq!(mapped["id"], json!(7));
        assert_eq!(mapped["progress"], json!(3));
        assert_eq!(mapped["media"]["id"], json!(1));
    }

    #[test]
    fn entry_mapping_rejects_bad_rows() {
        assert!(map_list_entry(&json!({ "id": 1, "status": "CURRENT", "media": { "id": 0 } })).is_none());
        assert!(map_list_entry(&json!({ "id": 1, "media": { "id": 2 } })).is_none());
        assert!(map_list_entry(&json!({ "id": 1, "status": "CURRENT" })).is_none());
    }

    #[test]
    fn sync_matrix() {
        // Mid-series, fresh session → CURRENT + startedAt.
        let input = compute_save_input(1, 5, Some(26), None, None, true).unwrap();
        assert_eq!(input["status"], json!("CURRENT"));
        assert_eq!(input["progress"], json!(5));
        assert!(input.get("startedAt").is_some());

        // Final episode completed → COMPLETED + completedAt, progress clamped.
        let input = compute_save_input(1, 26, Some(26), Some("CURRENT"), Some(9), true).unwrap();
        assert_eq!(input["status"], json!("COMPLETED"));
        assert_eq!(input["progress"], json!(26));
        assert!(input.get("completedAt").is_some());

        // Rewatching a completed show → REPEATING.
        let input = compute_save_input(1, 2, Some(26), Some("COMPLETED"), Some(9), true).unwrap();
        assert_eq!(input["status"], json!("REPEATING"));

        // Invalid ids → None.
        assert!(compute_save_input(0, 5, None, None, None, false).is_none());
        assert!(compute_save_input(1, 0, None, None, None, false).is_none());
    }
}
