import { AppUpdateCheckResult } from "./app-update-types";
import type { GithubIssuesListResult } from "./github-issues-types";
import type { SearchFilterDefinition, SearchFilterValues } from "./search-filters";

interface ThemeContext {
  toggle: () => Promise<boolean>;
  dark: () => Promise<void>;
  light: () => Promise<void>;
  system: () => Promise<boolean>;
  current: () => Promise<"dark" | "light" | "system">;
}

type StreamMode = "sub" | "dub";

type StreamProvider = "animeparadise" | "twitch" | "youtube";

interface Show {
  id: string;
  providerId: string;
  name: string;
  episodeCount: number;
  mode: StreamMode;
  hasSub?: boolean;
  hasDub?: boolean;
}

interface Bookmark {
  status: "watching" | "plan_to_watch" | "completed" | "rewatching" | "paused" | "dropped";
  show: Show;
  score?: number;
  startedAt?: Date;
  completedAt?: Date;
  progress: number;
  rewatchedCount: number;
  notes?: string;
}

interface ShowSearchResult {
  id: string;
  providerId: string;
  /** Source provider name (tagged by the backend when merging multi-provider results). */
  provider?: StreamProvider;
  title: {
    english?: string;
    romanji?: string;
    native?: string;
  };
  thumbnail: string | null;
  availableEpisodes?: {
    sub?: number;
    dub?: number;
    raw?: number;
  };
  score?: number;
  status?: string;
  type?: string;
  episodeDuration?: number;
  /** Live/video providers: viewers or views (replaces episode counts). */
  viewerCount?: number;
  views?: number;
  isLive?: boolean;
  category?: string;
  channelName?: string;
  startedAt?: string;
  profileImage?: string | null;
  season?: {
    quarter?: string;
    year?: number;
  };
}

interface Episode {
  id: string;
  providerId: string;
  /** Source provider name (tagged by the backend when merging multi-provider results). */
  provider?: StreamProvider;
  title: {
    english?: string;
    romanji?: string;
    native?: string;
  };
  thumbnail: string | null;
  index: number;
  mode: StreamMode;
  /** Raw episode token as returned by the provider (e.g. "live", a video id).
   *  `index` is numeric-only; non-numeric episodes would otherwise become NaN. */
  token?: string;
  /** Live/video providers: viewers or views (replaces episode numbers). */
  viewerCount?: number;
  views?: number;
  isLive?: boolean;
  category?: string;
}

interface StreamQualityOption {
  id: string;
  label: string;
  height?: number;
  bandwidth?: number;
}

interface StreamUrlResult {
  url: string;
  referer: string;
  subtitles?: Array<{
    url: string;
    language: string;
    format: string;
    default?: boolean;
  }>;
  qualities?: StreamQualityOption[];
  selectedQuality?: string;
  anilistMediaId?: number;
  /** Total media length in ms (YouTube VODs), for players that report none. */
  durationMs?: number | null;
}

interface TranscodeProgressResult {
  state: "idle" | "running" | "done" | "error";
  progressPercent: number | null;
  message: string;
}

interface ShowDetails {
  id: string;
  providerId: string;
  name: string;
  thumbnail: string | null;
  type: string;
  description?: string | null;
  anilistId?: string | null;
  viewerCount?: number;
  views?: number;
  isLive?: boolean;
  category?: string;
  profileImage?: string | null;
}

export interface AniListShowDetails {
  id?: number;
  streamingEpisodes?: Array<{
    thumbnail?: string | null;
    title?: string | null;
  }> | null;
  bannerImage?: string | null;
  coverImage?: {
    extraLarge?: string | null;
    large?: string | null;
    medium?: string | null;
    color?: string | null;
  } | null;
  description?: string | null;
  title?: {
    english?: string | null;
    romaji?: string | null;
    native?: string | null;
    userPreferred?: string | null;
  } | null;
  duration?: number | null;
  episodes?: number | null;
  averageScore?: number | null;
  season?: string | null;
  seasonYear?: number | null;
  status?: string | null;
  type?: string | null;
  format?: string | null;
  genres?: string[] | null;
  isAdult?: boolean | null;
  popularity?: number | null;
  chapters?: number | null;
  volumes?: number | null;
  startDate?: {
    year?: number | null;
    month?: number | null;
    day?: number | null;
  } | null;
  endDate?: {
    year?: number | null;
    month?: number | null;
    day?: number | null;
  } | null;
  nextAiringEpisode?: {
    airingAt: number;
    timeUntilAiring: number;
    episode: number;
  } | null;
  mediaListEntry?: {
    id: number;
    status: string;
    progress?: number | null;
    startedAt?: AniListFuzzyDate | null;
    completedAt?: AniListFuzzyDate | null;
  } | null;
  isFavourite?: boolean | null;
  studios?: Array<{
    isMain?: boolean | null;
    node: { id: number; name: string };
  }>;
}

export interface AniListMediaPageVariables {
  page?: number;
  id?: number;
  type?: string;
  isAdult?: boolean;
  search?: string;
  format?: string[];
  status?: string;
  countryOfOrigin?: string;
  source?: string;
  season?: string;
  seasonYear?: number;
  year?: string;
  onList?: boolean;
  yearLesser?: number;
  yearGreater?: number;
  episodeLesser?: number;
  episodeGreater?: number;
  durationLesser?: number;
  durationGreater?: number;
  chapterLesser?: number;
  chapterGreater?: number;
  volumeLesser?: number;
  volumeGreater?: number;
  licensedBy?: number[];
  isLicensed?: boolean;
  genres?: string[];
  excludedGenres?: string[];
  tags?: string[];
  excludedTags?: string[];
  minimumTagRank?: number;
  /** GraphQL expects `[MediaSort]`; a single string is normalized to a one-element array. */
  sort?: string[] | string;
}

export interface AniListMediaPageResult {
  pageInfo: {
    total?: number | null;
    perPage?: number | null;
    currentPage?: number | null;
    lastPage?: number | null;
    hasNextPage?: boolean | null;
  };
  media: AniListShowDetails[];
}

export type AniListIntegrationStatus = { connected: false } | { connected: true; username: string };

export type AniListMediaListStatus =
  | "CURRENT"
  | "PLANNING"
  | "COMPLETED"
  | "REPEATING"
  | "PAUSED"
  | "DROPPED";

export interface AniListFuzzyDate {
  year?: number | null;
  month?: number | null;
  day?: number | null;
}

export interface AniListListMedia {
  id: number;
  title?: {
    english?: string | null;
    romaji?: string | null;
    native?: string | null;
    userPreferred?: string | null;
  } | null;
  coverImage?: {
    extraLarge?: string | null;
    large?: string | null;
  } | null;
  bannerImage?: string | null;
  episodes?: number | null;
  averageScore?: number | null;
  season?: string | null;
  seasonYear?: number | null;
  status?: string | null;
}

export interface AniListListEntry {
  id: number;
  status: AniListMediaListStatus;
  progress: number;
  score?: number;
  repeat: number;
  startedAt?: AniListFuzzyDate | null;
  completedAt?: AniListFuzzyDate | null;
  media: AniListListMedia;
}

export type AniListFavouriteMedia = AniListListMedia & {
  isFavourite: boolean;
};

export interface AniListListPageResult {
  entries: AniListListEntry[];
  pageInfo: {
    total?: number | null;
    perPage?: number | null;
    currentPage?: number | null;
    lastPage?: number | null;
    hasNextPage?: boolean | null;
  };
}

export interface AniListFavouritesPageResult {
  media: AniListFavouriteMedia[];
  pageInfo: {
    total?: number | null;
    perPage?: number | null;
    currentPage?: number | null;
    lastPage?: number | null;
    hasNextPage?: boolean | null;
  };
}

export interface AniListSaveListEntryInput {
  id?: number;
  mediaId?: number;
  status?: AniListMediaListStatus;
  progress?: number;
  startedAt?: AniListFuzzyDate;
  completedAt?: AniListFuzzyDate;
}

export interface AniListSyncWatchProgressInput {
  mediaId: number;
  episodeNumber: number;
  totalEpisodes?: number;
  currentStatus?: AniListMediaListStatus | null;
  listEntryId?: number | null;
  episodeCompleted?: boolean;
}

interface HistoryEntry {
  id: string;
  provider: StreamProvider;
  episode: Episode;
  currentDurationMs: number;
  totalDurationMs: number;
  timestamp: number;
}

interface RecentlyWatchedContext {
  upsert: (entry: HistoryEntry) => Promise<void>;
  upsertSync: (entry: HistoryEntry) => void;
  read: () => Promise<HistoryEntry[]>;
  clear: () => Promise<void>;
  remove: (id: string) => Promise<void>;
}

interface PrivacyContext {
  getIncognito: () => Promise<boolean>;
  setIncognito: (enabled: boolean) => Promise<boolean>;
}

interface AppContext {
  version: () => Promise<string>;
  os: () => Promise<string>;
  /** True when required system dependencies are missing (e.g. Git Bash on Windows). */
  dependenciesRequired: () => Promise<boolean>;
  checkForUpdate: () => Promise<AppUpdateCheckResult>;
  /** Opens the app log directory in the system file manager. */
  openLogsDirectory: () => Promise<void>;
  /** Lists open GitHub issues for the app repository. */
  listGithubIssues: () => Promise<GithubIssuesListResult>;
  /** Lists pinned GitHub issues labeled `breaking`. */
  listPinnedGithubIssues: () => Promise<GithubIssuesListResult>;
}

interface WindowControlsContext {
  minimize: () => Promise<void>;
  close: () => Promise<void>;
  toggleMaximize: () => Promise<boolean>;
  isMaximized: () => Promise<boolean>;
  /** Manual drag fallback for setups where data-tauri-drag-region is ignored. */
  startDragging: () => Promise<void>;
}

export interface UrlOpenerContext {
  openUrl: (url: string) => Promise<void>;
}

interface StreamProviderContext {
  /** @deprecated Use getEnabledProviders; kept for back-compat (returns preferred provider). */
  getActiveProvider: () => Promise<StreamProvider>;
  /** @deprecated Use setEnabledProviders; kept for back-compat. */
  setActiveProvider: (provider: StreamProvider) => Promise<StreamProvider>;
  getEnabledProviders: () => Promise<StreamProvider[]>;
  setEnabledProviders: (providers: StreamProvider[]) => Promise<StreamProvider[]>;
  search: (query: string, filters?: SearchFilterValues) => Promise<ShowSearchResult[]>;
  getSearchFilters: () => Promise<SearchFilterDefinition[]>;
  getEpisodes: (
    providerId: string,
    mode?: "sub" | "dub",
    providerName?: StreamProvider
  ) => Promise<string[]>;
  getStreamUrl: (
    id: string | null,
    providerId: string | null,
    episode: string,
    mode?: "sub" | "dub",
    providerName?: StreamProvider
  ) => Promise<StreamUrlResult>;
  /** Re-resolve an open stream at a different quality (Twitch/YouTube progressive). */
  getStreamQualityUrl?: (
    providerId: string | null,
    episode: string,
    quality: string,
    providerName?: StreamProvider
  ) => Promise<StreamUrlResult>;
  getStreamProxyBaseUrl: () => Promise<string>;
  prepareTranscodedStream: (
    targetUrl: string,
    referer: string | null,
    variant?: string | null
  ) => Promise<boolean>;
  cancelTranscodedStream: (targetUrl: string, variant?: string | null) => Promise<boolean>;
  getTranscodeProgress: (
    targetUrl: string,
    variant?: string | null
  ) => Promise<TranscodeProgressResult>;
  getFfmpegStatus: () => Promise<{ available: boolean; path: string | null }>;
  getTranscodeLastError: (
    targetUrl: string,
    variant?: string | null
  ) => Promise<{ progress: TranscodeProgressResult; stderrTail: string }>;
  getShowDetails: (providerId: string, providerName?: StreamProvider) => Promise<ShowDetails>;
  getRecentUploads: (page: number, limit?: number, provider?: StreamProvider) => Promise<Episode[]>;
}

export interface AniListContext {
  getShowDetails: (mediaId: number) => Promise<AniListShowDetails>;
  search: (variables: AniListMediaPageVariables) => Promise<AniListMediaPageResult>;
  getPopularSeason: () => Promise<AniListShowDetails[]>;

  connect: () => Promise<{ ok: true } | { ok: false; error: string }>;
  disconnect: () => Promise<void>;
  getStatus: () => Promise<AniListIntegrationStatus>;
  openPinAuthPage: () => Promise<void>;
  submitManualToken: (token: string) => Promise<{ ok: true } | { ok: false; error: string }>;

  getMediaList: (
    status: AniListMediaListStatus,
    page?: number
  ) => Promise<AniListListPageResult>;
  getFavourites: (page?: number) => Promise<AniListFavouritesPageResult>;
  saveListEntry: (input: AniListSaveListEntryInput) => Promise<AniListListEntry>;
  deleteListEntry: (listEntryId: number) => Promise<void>;
  toggleFavourite: (mediaId: number) => Promise<boolean>;
  syncWatchProgress: (input: AniListSyncWatchProgressInput) => Promise<AniListListEntry | null>;
}

declare global {
  interface Window {
    app: AppContext;
    theme: ThemeContext;
    privacy: PrivacyContext;
    streamProvider: StreamProviderContext;
    anilist: AniListContext;
    recentlyWatched: RecentlyWatchedContext;
    windowControls: WindowControlsContext;
    urlOpener: UrlOpenerContext;
  }
}
