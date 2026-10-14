import { invoke } from "@tauri-apps/api/core";

/**
 * Tauri bridge that preserves the Electron `window.*` API surface 1:1,
 * so all existing pages/hooks/components work unchanged.
 * Imported once from renderer.ts before app.tsx renders.
 */
export function installTauriBridge() {
  const g = window as unknown as Record<string, unknown>;
  if (g.__openmediaTauriBridgeInstalled) return;
  g.__openmediaTauriBridgeInstalled = true;

  g.theme = {
    current: () => invoke("theme_current"),
    toggle: () => invoke("theme_toggle"),
    dark: () => invoke("theme_dark"),
    light: () => invoke("theme_light"),
    system: () => invoke("theme_system"),
  };

  g.windowControls = {
    minimize: () => invoke("window_minimize"),
    close: () => invoke("window_close"),
    toggleMaximize: () => invoke("window_toggle_maximize"),
    isMaximized: () => invoke("window_is_maximized"),
  };

  g.urlOpener = {
    openUrl: (url: string) => invoke("external_open_url", { url }),
  };

  g.app = {
    version: () => invoke("app_version"),
    os: () => invoke("app_os"),
    checkForUpdate: () => invoke("app_check_for_update"),
    openLogsDirectory: () => invoke("app_open_logs_directory"),
    listGithubIssues: () => invoke("app_list_github_issues"),
    listPinnedGithubIssues: () => invoke("app_list_pinned_github_issues"),
  };

  g.privacy = {
    getIncognito: () => invoke("privacy_incognito_get"),
    setIncognito: (enabled: boolean) => invoke("privacy_incognito_set", { enabled }),
  };

  g.recentlyWatched = {
    upsert: (entry: unknown) => invoke("recently_watched_upsert", { entry }),
    // Old Electron API was sendSync (blocking). Tauri is async-only;
    // fire-and-forget preserves call sites without behavior loss.
    upsertSync: (entry: unknown) => {
      void invoke("recently_watched_upsert", { entry });
    },
    read: () => invoke("recently_watched_read"),
    clear: () => invoke("recently_watched_clear"),
    remove: (id: string) => invoke("recently_watched_remove", { id }),
  };

  g.streamProvider = {
    getActiveProvider: () => invoke("stream_provider_active_get"),
    setActiveProvider: (provider: string) =>
      invoke("stream_provider_active_set", { provider }),
    search: (query: string, filters?: unknown) =>
      invoke("stream_provider_search", { query, filters: filters ?? null }),
    getSearchFilters: () => invoke("stream_provider_search_filters"),
    getEpisodes: (providerId: string, mode?: string, providerName?: string) =>
      invoke("stream_provider_episodes", {
        providerId,
        mode: mode ?? "sub",
        providerOverride: providerName ?? null,
      }),
    getStreamUrl: (
      id: string | null,
      providerId: string | null,
      episode: string,
      mode?: string,
      providerName?: string,
    ) =>
      invoke("stream_provider_stream_url", {
        id,
        providerId,
        episode,
        mode: mode ?? "sub",
        providerOverride: providerName ?? null,
      }),
    getStreamProxyBaseUrl: () => invoke("stream_provider_proxy_base"),
    prepareTranscodedStream: (targetUrl: string, referer: string | null, variant?: string | null) =>
      invoke("stream_provider_prepare_transcode", {
        targetUrl,
        referer,
        variant: variant ?? null,
      }),
    cancelTranscodedStream: (targetUrl: string, variant?: string | null) =>
      invoke("stream_provider_cancel_transcode", {
        targetUrl,
        variant: variant ?? null,
      }),
    getTranscodeProgress: (targetUrl: string, variant?: string | null) =>
      invoke("stream_provider_transcode_progress", {
        targetUrl,
        variant: variant ?? null,
      }),
    getShowDetails: (providerId: string, providerName?: string) =>
      invoke("stream_provider_show_details", {
        providerId,
        providerOverride: providerName ?? null,
      }),
    getRecentUploads: (page: number, limit?: number) =>
      invoke("stream_provider_recent_uploads", { page, limit: limit ?? null }),
  };

  g.anilist = {
    getShowDetails: (mediaId: number) => invoke("anilist_show_details", { mediaId }),
    search: (variables: unknown) => invoke("anilist_search", { variables }),
    getPopularSeason: () => invoke("anilist_popular_season"),
    connect: () => invoke("anilist_connect"),
    disconnect: () => invoke("anilist_disconnect"),
    getStatus: () => invoke("anilist_status"),
    openPinAuthPage: () => invoke("anilist_open_pin_auth"),
    submitManualToken: (token: string) => invoke("anilist_submit_manual_token", { token }),
    getMediaList: (status: string, page?: number) =>
      invoke("anilist_get_media_list", { status, page: page ?? null }),
    getFavourites: (page?: number) => invoke("anilist_get_favourites", { page: page ?? null }),
    saveListEntry: (input: unknown) => invoke("anilist_save_list_entry", { input }),
    deleteListEntry: (listEntryId: number) =>
      invoke("anilist_delete_list_entry", { listEntryId }),
    toggleFavourite: (mediaId: number) => invoke("anilist_toggle_favourite", { mediaId }),
    syncWatchProgress: (input: unknown) => invoke("anilist_sync_watch_progress", { input }),
  };
}
