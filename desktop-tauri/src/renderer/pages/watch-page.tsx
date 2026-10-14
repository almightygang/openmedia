import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router-dom";

import { DisabledStreamProviderDialog } from "@/renderer/components/disabled-stream-provider-dialog";
import type { PlayerSubtitleTrack } from "@/renderer/components/player/videojs-react-player";
import {
  WatchEpisodeSidebar,
  buildWatchSidebarEpisodes,
} from "@/renderer/components/player/watch-episode-sidebar";
import { WatchVideoPlayerShell } from "@/renderer/components/player/watch-video-player-shell";
import { Button } from "@/renderer/components/ui/button";
import { useGoBack } from "@/renderer/hooks/use-go-back";
import { isHistoryProviderDisabled } from "@/shared/stream-providers";
import type { AniListMediaListStatus, Episode, HistoryEntry } from "@/shared/types";
import { isHlsPlaylistUrl } from "@/shared/utils/hls-url";

import { type RichShowDetails, useShowDetails } from "../hooks/use-show-details";

/** How many automatic reconnects after a playback error before showing the manual overlay. */
const MAX_AUTO_RECONNECT = 3;

/** Frontend guard so a wedged backend can't leave the UI on "Preparing..." forever. */
const PREPARE_TIMEOUT_MS = 60_000;

const IS_DEV = process.env.NODE_ENV !== "production";
const ENABLE_HLS_SERVER_TRANSCODE = true;

function logPlaybackFailure(event: string, meta?: Record<string, unknown>): void {
  if (!IS_DEV) return;
  const suffix = meta ? ` ${JSON.stringify(meta)}` : "";
  console.error(`[watch-playback] ${event}${suffix}`);
}

function shouldUseServerTranscode(streamUrl: string, provider?: string): boolean {
  if (!ENABLE_HLS_SERVER_TRANSCODE) return false;
  // Twitch resolves to already-playable live HLS; ffmpeg only adds latency
  // and races expiring signed segment URLs.
  if (provider === "twitch") return false;
  if (provider === "youtube") {
    // Live HLS plays direct; progressive MP4 VODs go through the remux
    // pipeline so the player gets exact duration + full seeking (native
    // MP4 often reports unknown length, clamping seeks to buffered ranges).
    return !isHlsPlaylistUrl(streamUrl);
  }
  return isHlsPlaylistUrl(streamUrl);
}

function subtitleLangCode(label: string): string {
  const paren = /\(([A-Za-z]{2,3})\)\s*$/.exec(label);
  if (paren?.[1]) return paren[1].toLowerCase();
  if (/english/i.test(label)) return "en";
  if (/arabic/i.test(label)) return "ar";
  if (/french/i.test(label)) return "fr";
  if (/german/i.test(label)) return "de";
  if (/italian/i.test(label)) return "it";
  if (/spanish|castilian/i.test(label)) return "es";
  if (/portuguese/i.test(label)) return "pt";
  if (/russian/i.test(label)) return "ru";
  if (/polish/i.test(label)) return "pl";
  return "und";
}

function proxySubtitleTracks(
  base: string,
  referer: string,
  subtitles: Array<{ url: string; language: string; format: string; default?: boolean }> | undefined
): PlayerSubtitleTrack[] {
  if (!subtitles?.length) return [];
  return subtitles.map((track) => ({
    src: `${base}/stream?url=${encodeURIComponent(track.url)}&referer=${encodeURIComponent(referer)}`,
    label: track.language,
    srclang: subtitleLangCode(track.language),
    default: track.default === true,
  }));
}

function buildWatchHistoryEntry(
  episode: Episode,
  ep: string,
  provider: HistoryEntry["provider"],
  showDetails: RichShowDetails | null,
  mode: Episode["mode"]
): HistoryEntry {
  const parsed = Number(ep);
  const index = Number.isFinite(parsed) ? parsed : (episode.index ?? 0);
  const richList = showDetails?.episodes[mode] ?? [];
  const rich = richList.find((e) => e.index === index);
  const thumb =
    rich?.thumbnail ?? showDetails?.coverImage ?? showDetails?.bannerImage ?? episode.thumbnail;

  return {
    id: `${provider}:${episode.id || episode.providerId}-${ep}`,
    provider,
    episode: {
      ...episode,
      index,
      token: ep,
      title: {
        english: showDetails?.title.english ?? episode.title.english,
        romanji: showDetails?.title.romaji ?? episode.title.romanji,
        native: showDetails?.title.native ?? episode.title.native,
      },
      thumbnail: thumb,
      mode,
    },
    currentDurationMs: 0,
    totalDurationMs: 0,
    timestamp: Date.now(),
  };
}

interface WatchState {
  episode: Episode;
  /** Saved position from Continue watching (ms). */
  resumeFromMs?: number;
  /** Optional stream provider override for this watch session only. */
  providerOverride?: HistoryEntry["provider"];
}

type WatchKind = "anime" | "live" | "video";

function resolveWatchKind(
  episode: Episode | undefined,
  providerOverride: HistoryEntry["provider"] | undefined,
  showIsLive?: boolean | null
): WatchKind {
  const provider = providerOverride ?? episode?.provider;
  if (provider === "twitch") return "live";
  if (provider === "youtube") {
    const live = showIsLive ?? episode?.isLive ?? false;
    return live ? "live" : "video";
  }
  return "anime";
}

/** Raw provider episode token ("live", video id, "3") — never Number()-ed. */
function episodeToken(episode: Episode): string {
  if (episode.token != null && episode.token !== "") return episode.token;
  if (Number.isFinite(episode.index)) return String(episode.index);
  return "";
}

export function WatchPage() {
  const location = useLocation();
  const goBack = useGoBack();
  const state = location.state as WatchState | null;
  const resumeFromMs = state?.resumeFromMs;
  const streamProviderOverride = state?.providerOverride;
  const disabledHistoryProvider =
    streamProviderOverride && isHistoryProviderDisabled(streamProviderOverride)
      ? streamProviderOverride
      : null;

  const [playUrl, setPlayUrl] = useState<string>("");
  /** Bumps when a new stream URL is ready so the <video> remounts (retry after errors). */
  const [streamRevision, setStreamRevision] = useState(0);
  const [subtitleTracks, setSubtitleTracks] = useState<PlayerSubtitleTrack[]>([]);
  const [streamQualities, setStreamQualities] = useState<
    Array<{ id: string; label: string; height?: number; bandwidth?: number }>
  >([]);
  const [selectedQuality, setSelectedQuality] = useState<string | null>(null);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [loadingEpisode, setLoadingEpisode] = useState(false);
  /** Backend-reported total length (YouTube VODs) when the element reports none. */
  const streamDurationMsRef = useRef<number | null>(null);
  const [transcodeProgress, setTranscodeProgress] = useState<{
    active: boolean;
    progressPercent: number | null;
    message: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const episode = state?.episode;
  // Home rows carry no explicit override — fall back to the row's own tagged
  // provider so Twitch/YouTube entries don't resolve against AnimeParadise.
  const activeProvider = streamProviderOverride ?? episode?.provider;

  const {
    details: showDetails,
    episodesByMode,
    loading: showLoading,
    error: showError,
  } = useShowDetails(
    disabledHistoryProvider ? undefined : episode?.id,
    disabledHistoryProvider ? undefined : episode?.providerId,
    disabledHistoryProvider ? undefined : activeProvider
  );

  const [currentEpisode, setCurrentEpisode] = useState<number>(() => episode?.index ?? 1);
  const [currentToken, setCurrentToken] = useState<string>(() => (episode ? episodeToken(episode) : ""));
  const [playbackMode, setPlaybackMode] = useState<Episode["mode"]>(() => episode?.mode ?? "sub");
  const kind: WatchKind = resolveWatchKind(episode, activeProvider, showDetails?.isLive);
  const isLiveKind = kind !== "anime";
  const playbackModeRef = useRef(playbackMode);
  playbackModeRef.current = playbackMode;

  const sidebarEpisodes = useMemo(
    () =>
      buildWatchSidebarEpisodes(
        showDetails,
        episodesByMode,
        episode?.thumbnail ?? showDetails?.coverImage ?? showDetails?.bannerImage
      ),
    [showDetails, episodesByMode, episode?.thumbnail]
  );

  const videoRef = useRef<HTMLVideoElement | null>(null);
  const lastHistoryEntryRef = useRef<HistoryEntry | null>(null);
  /** Upstream stream URL currently being transcoded (for cancel-on-exit). */
  const activeTranscodeUrlRef = useRef<string | null>(null);
  const activeTranscodeVariantRef = useRef<string | null>(null);
  const activeStreamRef = useRef<{ url: string; referer: string } | null>(null);
  /** Seconds to seek to after the next successful load (set before reload). */
  const resumeAfterLoadRef = useRef<number | null>(null);
  /** Last known position when playback failed (for manual retry after overlay). */
  const lastPlaybackTimeRef = useRef<number | null>(null);
  const autoReconnectAttemptRef = useRef(0);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** Stream revision when load succeeded; in-memory entry is enriched after show details load. */
  const deferredHistoryUpsertRef = useRef<{ revision: number; ep: string } | null>(null);
  const activeLoadTokenRef = useRef(0);
  const historyProviderRef = useRef<HistoryEntry["provider"]>("animeparadise");
  const anilistListRef = useRef<{
    mediaId: number;
    listEntryId?: number | null;
    currentStatus?: AniListMediaListStatus | null;
    totalEpisodes?: number;
  } | null>(null);
  const anilistSyncedEpisodeRef = useRef<number | null>(null);
  const incognitoRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    void window.privacy.getIncognito().then((enabled) => {
      if (!cancelled) {
        incognitoRef.current = enabled;
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const clearReconnectTimeout = useCallback(() => {
    if (reconnectTimeoutRef.current != null) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }
  }, []);

  const syncAniListProgress = useCallback(
    async (episodeNumber: number, episodeCompleted = false) => {
      if (incognitoRef.current) return;
      const ctx = anilistListRef.current;
      if (!ctx) return;
      if (!episodeCompleted && anilistSyncedEpisodeRef.current === episodeNumber) return;

      try {
        const status = await window.anilist.getStatus();
        if (!status.connected) return;

        const entry = await window.anilist.syncWatchProgress({
          mediaId: ctx.mediaId,
          episodeNumber,
          totalEpisodes: ctx.totalEpisodes,
          currentStatus: ctx.currentStatus,
          listEntryId: ctx.listEntryId,
          episodeCompleted,
        });
        if (entry) {
          anilistListRef.current = {
            ...ctx,
            listEntryId: entry.id,
            currentStatus: entry.status,
          };
        }
        if (!episodeCompleted) {
          anilistSyncedEpisodeRef.current = episodeNumber;
        }
      } catch {
        // best-effort
      }
    },
    []
  );

  const syncHistoryProgress = useCallback(async (opts?: { sync?: boolean }) => {
    if (incognitoRef.current) return;
    const base = lastHistoryEntryRef.current;
    const video = videoRef.current;
    if (!base || !video) return;
    const currentMs = Math.max(0, Math.floor(video.currentTime * 1000));
    let totalMs = base.totalDurationMs;
    if (Number.isFinite(video.duration) && video.duration > 0) {
      totalMs = Math.floor(video.duration * 1000);
    } else if (streamDurationMsRef.current != null && streamDurationMsRef.current > 0) {
      totalMs = streamDurationMsRef.current;
    }
    const next: HistoryEntry = {
      ...base,
      currentDurationMs: currentMs,
      totalDurationMs: totalMs,
      timestamp: Date.now(),
    };
    lastHistoryEntryRef.current = next;
    try {
      if (opts?.sync) {
        window.recentlyWatched.upsertSync(next);
        return;
      }
      await window.recentlyWatched.upsert(next);
    } catch {
      // best-effort
    }
  }, []);

  const applyResumeIfNeeded = useCallback((video: HTMLVideoElement) => {
    const resume = resumeAfterLoadRef.current;
    if (resume == null || resume <= 0 || Number.isNaN(resume)) return;
    resumeAfterLoadRef.current = null;
    const dur = video.duration;
    let target = resume;
    if (Number.isFinite(dur) && dur > 0) {
      target = Math.min(resume, Math.max(0, dur - 0.25));
    }
    video.currentTime = Math.max(0, target);
  }, []);

  const handleVideoPlaying = useCallback(() => {
    autoReconnectAttemptRef.current = 0;
  }, []);

  const handleVideoPause = useCallback(() => {
    void syncHistoryProgress();
  }, [syncHistoryProgress]);

  const handleVideoEnded = useCallback(() => {
    void syncHistoryProgress();
    void syncAniListProgress(currentEpisode, true);
  }, [syncHistoryProgress, syncAniListProgress, currentEpisode]);

  const handleVideoLoadedMetadata = useCallback<React.ReactEventHandler<HTMLVideoElement>>(
    (e) => {
      applyResumeIfNeeded(e.currentTarget);
      void syncHistoryProgress();
    },
    [applyResumeIfNeeded, syncHistoryProgress]
  );

  const loadStream = useCallback(
    async (ep: string, opts?: { resumeFrom?: number | null; mode?: Episode["mode"] }) => {
      if (!episode?.providerId || !ep) return;
      const mode = opts?.mode ?? playbackModeRef.current;
      playbackModeRef.current = mode;
      setPlaybackMode(mode);
      activeLoadTokenRef.current += 1;
      const loadToken = activeLoadTokenRef.current;
      const parsedEp = Number(ep);
      setCurrentEpisode(Number.isFinite(parsedEp) ? parsedEp : 0);
      setCurrentToken(ep);
      setPlayUrl("");
      lastHistoryEntryRef.current = null;
      clearReconnectTimeout();
      if (opts?.resumeFrom != null && opts.resumeFrom > 0) {
        resumeAfterLoadRef.current = opts.resumeFrom;
      } else {
        resumeAfterLoadRef.current = null;
      }
      setLoadingEpisode(true);
      setTranscodeProgress(null);
      setError(null);
      setPlaybackError(null);
      setSubtitleTracks([]);
      setStreamQualities([]);
      setSelectedQuality(null);
      activeStreamRef.current = null;
      const previousTranscodeUrl = activeTranscodeUrlRef.current;
      const previousTranscodeVariant = activeTranscodeVariantRef.current;
      if (previousTranscodeUrl) {
        activeTranscodeUrlRef.current = null;
        activeTranscodeVariantRef.current = null;
        void window.streamProvider.cancelTranscodedStream(
          previousTranscodeUrl,
          previousTranscodeVariant
        );
      }
      try {
        const {
          url,
          referer,
          subtitles,
          qualities,
          selectedQuality: defaultQuality,
          anilistMediaId,
          durationMs,
        } = await window.streamProvider.getStreamUrl(
          episode.id,
          episode.providerId,
          ep,
          mode,
          activeProvider
        );
        streamDurationMsRef.current =
          typeof durationMs === "number" && Number.isFinite(durationMs) && durationMs > 0
            ? Math.floor(durationMs)
            : null;
        if (anilistMediaId != null && Number.isInteger(anilistMediaId) && anilistMediaId > 0) {
          const prev = anilistListRef.current;
          if (prev?.mediaId !== anilistMediaId) {
            anilistListRef.current = { mediaId: anilistMediaId };
          }
        }
        historyProviderRef.current =
          streamProviderOverride ??
          episode.provider ??
          (await window.streamProvider.getEnabledProviders()).at(0) ??
          (await window.streamProvider.getActiveProvider());
        const base = await window.streamProvider.getStreamProxyBaseUrl();
        setSubtitleTracks(proxySubtitleTracks(base, referer, subtitles));
        const qualityOptions = qualities ?? [];
        const variant =
          (defaultQuality && qualityOptions.some((q) => q.id === defaultQuality)
            ? defaultQuality
            : qualityOptions[0]?.id) ?? null;
        setStreamQualities(qualityOptions);
        setSelectedQuality(variant);
        activeStreamRef.current = { url, referer };
        const shouldTranscode = shouldUseServerTranscode(url, activeProvider);
        if (shouldTranscode) {
          activeTranscodeUrlRef.current = url;
          activeTranscodeVariantRef.current = variant;
          setTranscodeProgress({
            active: true,
            progressPercent: 0,
            message: "Connecting to stream...",
          });

          let stopPolling = false;
          const pollProgress = async () => {
            try {
              const next = await window.streamProvider.getTranscodeProgress(url, variant);
              if (stopPolling || activeLoadTokenRef.current !== loadToken) return;
              setTranscodeProgress({
                active: next.state === "running" || next.state === "idle",
                progressPercent: next.progressPercent,
                message: next.message || "Transcoding video...",
              });
            } catch {
              // best-effort
            }
          };

          await pollProgress();
          const progressInterval = setInterval(() => {
            void pollProgress();
          }, 500);
          try {
            const prepare = window.streamProvider.prepareTranscodedStream(url, referer, variant);
            const timeout = new Promise<never>((_, reject) => {
              setTimeout(
                () => reject(new Error("Timed out preparing the stream (60s). The server may still be transcoding — try again.")),
                PREPARE_TIMEOUT_MS
              );
            });
            await Promise.race([prepare, timeout]);
          } catch (prepareErr) {
            stopPolling = true;
            clearInterval(progressInterval);
            if (activeLoadTokenRef.current !== loadToken) {
              return;
            }
            // Don't leave a wedged job running behind an error screen.
            if (activeTranscodeUrlRef.current === url) {
              activeTranscodeUrlRef.current = null;
              activeTranscodeVariantRef.current = null;
              void window.streamProvider.cancelTranscodedStream(url, variant);
            }
            throw prepareErr;
          } finally {
            stopPolling = true;
            clearInterval(progressInterval);
          }
          if (activeLoadTokenRef.current !== loadToken) {
            if (activeTranscodeUrlRef.current === url) {
              activeTranscodeUrlRef.current = null;
              activeTranscodeVariantRef.current = null;
              void window.streamProvider.cancelTranscodedStream(url, variant);
            }
            return;
          }
          setTranscodeProgress({
            active: false,
            progressPercent: 100,
            message: "Ready to play",
          });
        }

        const variantQuery =
          variant && variant.trim() ? `&variant=${encodeURIComponent(variant.trim())}` : "";
        const urlWithProxy = shouldTranscode
          ? `${base}/transcode/playlist.m3u8?url=${encodeURIComponent(url)}&referer=${encodeURIComponent(referer)}${variantQuery}`
          : isHlsPlaylistUrl(url)
            ? `${base}/stream/playlist.m3u8?url=${encodeURIComponent(url)}&referer=${encodeURIComponent(referer)}${variantQuery}`
            : `${base}/stream?url=${encodeURIComponent(url)}&referer=${encodeURIComponent(referer)}`;
        if (activeLoadTokenRef.current !== loadToken) return;
        setPlayUrl(urlWithProxy);
        setStreamRevision((r) => {
          const next = r + 1;
          deferredHistoryUpsertRef.current = { revision: next, ep };
          return next;
        });
        lastHistoryEntryRef.current = buildWatchHistoryEntry(
          episode,
          ep,
          historyProviderRef.current,
          null,
          mode
        );
        const epNum = Number(ep);
        // Live/video providers are not anime: never sync them to AniList
        // (Number("live") is NaN and YouTube ids are not AniList media).
        if (activeProvider !== "twitch" && activeProvider !== "youtube" && Number.isFinite(epNum)) {
          void syncAniListProgress(epNum);
        }
      } catch (err) {
        if (activeLoadTokenRef.current !== loadToken) return;
        const raw = err instanceof Error ? err.message : "Failed to load stream";
        const hint = /ffmpeg binary unavailable/i.test(raw)
          ? " (ffmpeg sidecar missing — reinstall the app or put ffmpeg on PATH)"
          : /first video segment|timed out preparing/i.test(raw)
            ? " If this repeats, the source may be slow/blocked or ffmpeg failed — Retry, or check logs."
            : "";
        setError(`${raw}${hint}`);
      } finally {
        if (activeLoadTokenRef.current === loadToken) {
          setLoadingEpisode(false);
          setTranscodeProgress(null);
        }
      }
    },
    [episode, clearReconnectTimeout, streamProviderOverride, activeProvider, syncAniListProgress]
  );

  useEffect(() => {
    if (!episode?.id) {
      anilistListRef.current = null;
      anilistSyncedEpisodeRef.current = null;
      return;
    }

    // Prefer a resolved AniList id from show details; never assume episode.id is AniList
    // (provider-native numeric ids — e.g. anidb.app — collide with AniList media ids).
    const fromEpisode = Number(episode.id);
    const mediaId =
      Number.isInteger(fromEpisode) && fromEpisode > 0 && episode.id !== episode.providerId
        ? fromEpisode
        : null;

    if (mediaId == null) {
      anilistListRef.current = null;
      return;
    }

    anilistListRef.current = { mediaId };

    let cancelled = false;
    void (async () => {
      try {
        const status = await window.anilist.getStatus();
        if (!status.connected || cancelled) return;
        const details = await window.anilist.getShowDetails(mediaId);
        if (cancelled) return;
        const listEntry = details.mediaListEntry;
        anilistListRef.current = {
          mediaId,
          listEntryId: listEntry?.id,
          currentStatus: listEntry?.status as AniListMediaListStatus | undefined,
          totalEpisodes: details.episodes ?? undefined,
        };
      } catch {
        anilistListRef.current = { mediaId };
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [episode?.id, episode?.providerId]);

  useEffect(() => {
    if (showLoading || !showDetails) return;
    const mediaId = showDetails.anilistMediaId;
    if (mediaId == null) return;

    const prev = anilistListRef.current;
    const next = {
      mediaId,
      listEntryId: showDetails.anilistListEntry?.id ?? prev?.listEntryId,
      currentStatus:
        (showDetails.anilistListEntry?.status as AniListMediaListStatus | undefined) ??
        prev?.currentStatus,
      totalEpisodes: prev?.totalEpisodes,
    };
    const subCount = showDetails.episodes.sub?.length ?? 0;
    const dubCount = showDetails.episodes.dub?.length ?? 0;
    const totalEpisodes = Math.max(subCount, dubCount);
    if (totalEpisodes > 0) {
      next.totalEpisodes = totalEpisodes;
    }
    anilistListRef.current = next;
    if (playUrl) {
      void syncAniListProgress(currentEpisode);
    }
  }, [showLoading, showDetails, playUrl, currentEpisode, syncAniListProgress]);

  useEffect(() => {
    if (!episode) return;
    setCurrentEpisode(episode.index);
    setCurrentToken(episodeToken(episode));
    playbackModeRef.current = episode.mode;
    setPlaybackMode(episode.mode);
  }, [episode?.id, episode?.providerId, episode?.index, episode?.mode]);

  useEffect(() => {
    if (!episode || disabledHistoryProvider) return;
    const resumeFrom = resumeFromMs != null && resumeFromMs > 0 ? resumeFromMs / 1000 : undefined;
    void loadStream(episodeToken(episode), resumeFrom != null ? { resumeFrom } : undefined);
  }, [
    disabledHistoryProvider,
    episode?.id,
    episode?.providerId,
    episode?.index,
    episode?.mode,
    loadStream,
    resumeFromMs,
  ]);

  useEffect(() => {
    if (showLoading || !episode || !playUrl) return;
    const pending = deferredHistoryUpsertRef.current;
    if (!pending || pending.revision !== streamRevision) return;

    const prev = lastHistoryEntryRef.current;
    const entry = buildWatchHistoryEntry(
      episode,
      pending.ep,
      historyProviderRef.current,
      showDetails,
      playbackModeRef.current
    );
    const video = videoRef.current;
    if (video && !Number.isNaN(video.currentTime) && video.currentTime > 0) {
      entry.currentDurationMs = Math.max(
        entry.currentDurationMs,
        Math.floor(video.currentTime * 1000)
      );
    }
    if (video && Number.isFinite(video.duration) && video.duration > 0) {
      entry.totalDurationMs = Math.floor(video.duration * 1000);
    } else if (streamDurationMsRef.current != null && streamDurationMsRef.current > 0) {
      entry.totalDurationMs = streamDurationMsRef.current;
    } else if (prev && prev.id === entry.id && prev.totalDurationMs > 0) {
      entry.totalDurationMs = prev.totalDurationMs;
    }
    lastHistoryEntryRef.current = entry;
    deferredHistoryUpsertRef.current = null;
  }, [showLoading, showDetails, playUrl, episode, streamRevision]);

  useEffect(() => {
    return () => {
      clearReconnectTimeout();
      activeLoadTokenRef.current += 1;
      const transcodeUrl = activeTranscodeUrlRef.current;
      const transcodeVariant = activeTranscodeVariantRef.current;
      activeTranscodeUrlRef.current = null;
      activeTranscodeVariantRef.current = null;
      if (transcodeUrl) {
        void window.streamProvider.cancelTranscodedStream(transcodeUrl, transcodeVariant);
      }
    };
  }, [clearReconnectTimeout]);

  const onEpisodeSelect = useCallback(
    (ep: string, mode?: Episode["mode"]) => {
      const nextMode = mode ?? playbackModeRef.current;
      if (ep === currentToken && nextMode === playbackModeRef.current) return;
      const parsed = Number(ep);
      setCurrentEpisode(Number.isFinite(parsed) ? parsed : 0);
      setCurrentToken(ep);
      anilistSyncedEpisodeRef.current = null;
      void (async () => {
        await syncHistoryProgress();
        await loadStream(ep, { mode: nextMode });
      })();
    },
    [currentToken, loadStream, syncHistoryProgress]
  );

  const onSidebarSelect = useCallback(
    (index: number, mode: Episode["mode"]) => {
      onEpisodeSelect(String(index), mode);
    },
    [onEpisodeSelect]
  );

  const onQualitySelect = useCallback(
    (qualityId: string) => {
      const stream = activeStreamRef.current;
      if (!stream || !qualityId || qualityId === selectedQuality) return;

      void (async () => {
        activeLoadTokenRef.current += 1;
        const loadToken = activeLoadTokenRef.current;
        const resumeFrom = videoRef.current?.currentTime;
        if (resumeFrom != null && resumeFrom > 0) {
          resumeAfterLoadRef.current = resumeFrom;
        }

        const previousUrl = activeTranscodeUrlRef.current;
        const previousVariant = activeTranscodeVariantRef.current;
        if (previousUrl) {
          activeTranscodeUrlRef.current = null;
          activeTranscodeVariantRef.current = null;
          void window.streamProvider.cancelTranscodedStream(previousUrl, previousVariant);
        }

        setSelectedQuality(qualityId);
        setLoadingEpisode(true);
        setPlaybackError(null);
        setError(null);
        setPlayUrl("");

        try {
          const { url, referer } = stream;
          const base = await window.streamProvider.getStreamProxyBaseUrl();
          const shouldTranscode = shouldUseServerTranscode(url, activeProvider);
          const isHls = isHlsPlaylistUrl(url);
          // Progressive (MP4) sources: each quality is its own URL, so
          // re-resolve at the requested quality, then remux through the
          // transcode pipeline (exact duration + full seeking, like anime).
          if (!isHls && window.streamProvider.getStreamQualityUrl && episode) {
            const resolved = await window.streamProvider.getStreamQualityUrl(
              episode.providerId,
              currentToken || episodeToken(episode),
              qualityId,
              activeProvider
            );
            if (activeLoadTokenRef.current !== loadToken) return;
            activeStreamRef.current = {
              url: resolved.url,
              referer: resolved.referer,
            };
            if (
              typeof resolved.durationMs === "number" &&
              Number.isFinite(resolved.durationMs) &&
              resolved.durationMs > 0
            ) {
              streamDurationMsRef.current = Math.floor(resolved.durationMs);
            }
            setStreamQualities(resolved.qualities ?? streamQualities);
            setSelectedQuality(
              resolved.selectedQuality && (resolved.qualities ?? []).some((q) => q.id === resolved.selectedQuality)
                ? resolved.selectedQuality
                : qualityId
            );
            activeTranscodeUrlRef.current = resolved.url;
            activeTranscodeVariantRef.current = null;
            setTranscodeProgress({
              active: true,
              progressPercent: 0,
              message: "Switching quality...",
            });
            await Promise.race([
              window.streamProvider.prepareTranscodedStream(resolved.url, resolved.referer, null),
              new Promise<never>((_, reject) => {
                setTimeout(
                  () => reject(new Error("Timed out switching quality (60s). Try again.")),
                  PREPARE_TIMEOUT_MS
                );
              }),
            ]);
            if (activeLoadTokenRef.current !== loadToken) return;
            setTranscodeProgress({
              active: false,
              progressPercent: 100,
              message: "Ready to play",
            });
            const remuxed = `${base}/transcode/playlist.m3u8?url=${encodeURIComponent(resolved.url)}&referer=${encodeURIComponent(resolved.referer)}`;
            setPlayUrl(remuxed);
            setStreamRevision((r) => r + 1);
            return;
          }
          if (shouldTranscode) {
            activeTranscodeUrlRef.current = url;
            activeTranscodeVariantRef.current = qualityId;
            setTranscodeProgress({
              active: true,
              progressPercent: 0,
              message: "Switching quality...",
            });
            await Promise.race([
              window.streamProvider.prepareTranscodedStream(url, referer, qualityId),
              new Promise<never>((_, reject) => {
                setTimeout(
                  () => reject(new Error("Timed out switching quality (60s). Try again.")),
                  PREPARE_TIMEOUT_MS
                );
              }),
            ]);
            if (activeLoadTokenRef.current !== loadToken) return;
            setTranscodeProgress({
              active: false,
              progressPercent: 100,
              message: "Ready to play",
            });
          }

          const variantQuery = `&variant=${encodeURIComponent(qualityId)}`;
          const urlWithProxy = shouldTranscode
            ? `${base}/transcode/playlist.m3u8?url=${encodeURIComponent(url)}&referer=${encodeURIComponent(referer)}${variantQuery}`
            : isHlsPlaylistUrl(url)
              ? `${base}/stream/playlist.m3u8?url=${encodeURIComponent(url)}&referer=${encodeURIComponent(referer)}${variantQuery}`
              : `${base}/stream?url=${encodeURIComponent(url)}&referer=${encodeURIComponent(referer)}`;
          if (activeLoadTokenRef.current !== loadToken) return;
          setPlayUrl(urlWithProxy);
          setStreamRevision((r) => r + 1);
        } catch (err) {
          if (activeLoadTokenRef.current !== loadToken) return;
          setError(err instanceof Error ? err.message : "Failed to switch quality");
        } finally {
          if (activeLoadTokenRef.current === loadToken) {
            setLoadingEpisode(false);
            setTranscodeProgress(null);
          }
        }
      })();
    },
    [selectedQuality, episode, episode?.provider, streamProviderOverride, activeProvider, streamQualities]
  );

  const retryStream = useCallback(() => {
    if (!currentToken) return;
    clearReconnectTimeout();
    autoReconnectAttemptRef.current = 0;
    const el = videoRef.current;
    const fromVideo =
      el && !Number.isNaN(el.currentTime) && el.currentTime > 0 ? el.currentTime : null;
    const resumeFrom = fromVideo ?? lastPlaybackTimeRef.current ?? undefined;
    setPlayUrl("");
    setPlaybackError(null);
    setError(null);
    void loadStream(currentToken, resumeFrom != null ? { resumeFrom } : undefined);
  }, [clearReconnectTimeout, currentToken, loadStream]);

  const handleVideoError = useCallback(() => {
    const el = videoRef.current;
    const mediaError = el?.error;
    logPlaybackFailure("video-element-error", {
      currentTime: el?.currentTime ?? null,
      readyState: el?.readyState ?? null,
      networkState: el?.networkState ?? null,
      mediaErrorCode: mediaError?.code ?? null,
      mediaErrorMessage: mediaError?.message ?? null,
    });
    const t = el && !Number.isNaN(el.currentTime) && el.currentTime > 0 ? el.currentTime : 0;
    lastPlaybackTimeRef.current = t > 0 ? t : lastPlaybackTimeRef.current;

    const ep = currentToken;

    autoReconnectAttemptRef.current += 1;
    const n = autoReconnectAttemptRef.current;

    if (n <= MAX_AUTO_RECONNECT) {
      // Give an in-flight transcode job a chance to produce seg-0 instead of
      // killing it instantly in a restart loop.
      const delayMs = Math.min(1500 * 2 ** (n - 1), 8000);
      clearReconnectTimeout();
      setPlayUrl("");
      setPlaybackError(null);
      const run = () => {
        void loadStream(ep, t > 0 ? { resumeFrom: t } : undefined);
      };
      reconnectTimeoutRef.current = setTimeout(() => {
        reconnectTimeoutRef.current = null;
        run();
      }, delayMs);
      return;
    }

    autoReconnectAttemptRef.current = 0;
    setPlaybackError(
      "Stream interrupted (e.g. network lost or server error). Reconnect automatically failed; try again or check your connection."
    );
  }, [clearReconnectTimeout, currentToken, loadStream]);

  useEffect(() => {
    return () => {
      void syncHistoryProgress({ sync: true });
    };
  }, [syncHistoryProgress]);

  useEffect(() => {
    const flushSyncOnExit = () => {
      void syncHistoryProgress({ sync: true });
    };
    const flushAsyncOnHidden = () => {
      void syncHistoryProgress();
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        flushAsyncOnHidden();
      }
    };
    window.addEventListener("beforeunload", flushSyncOnExit);
    window.addEventListener("pagehide", flushSyncOnExit);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.removeEventListener("beforeunload", flushSyncOnExit);
      window.removeEventListener("pagehide", flushSyncOnExit);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [syncHistoryProgress]);

  if (!episode) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8 min-h-0">
        <p className="text-muted-foreground">
          No episode selected. Go back and pick an episode to play.
        </p>
        <Button type="button" variant="outline" onClick={goBack}>
          Go back
        </Button>
      </div>
    );
  }

  if (disabledHistoryProvider) {
    return (
      <>
        <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8 min-h-0">
          <p className="text-muted-foreground">This watch history entry can&apos;t be resumed.</p>
          <Button type="button" variant="outline" onClick={goBack}>
            Go back
          </Button>
        </div>
        <DisabledStreamProviderDialog
          provider={disabledHistoryProvider}
          open
          onOpenChange={(open) => {
            if (!open) {
              goBack();
            }
          }}
        />
      </>
    );
  }

  const displayName =
    showDetails?.title.english ??
    showDetails?.title.romaji ??
    showDetails?.title.native ??
    episode.title.english ??
    episode.title.romanji ??
    episode.title.native ??
    "Unknown";

  const viewsCount = showDetails?.views ?? showDetails?.viewerCount ?? null;
  const headerLabel = isLiveKind
    ? kind === "live"
      ? viewsCount != null
        ? `${displayName} · LIVE · ${viewsCount >= 1000 ? `${(viewsCount / 1000).toFixed(1).replace(/\.0$/, "")}K` : viewsCount} watching`
        : `${displayName} · LIVE`
      : viewsCount != null && viewsCount > 0
        ? `${displayName} · ${viewsCount >= 1000 ? `${(viewsCount / 1000).toFixed(1).replace(/\.0$/, "")}K` : viewsCount} views`
        : displayName
    : displayName;

  return (
    <div className="flex h-full min-h-0 w-full flex-1 flex-col lg:flex-row">
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <WatchVideoPlayerShell
          playUrl={playUrl}
          streamRevision={streamRevision}
          loadingEpisode={loadingEpisode}
          transcodeProgress={transcodeProgress}
          streamError={error}
          playbackError={playbackError}
          displayName={displayName}
          headerLabel={headerLabel}
          isDub={playbackMode === "dub"}
          hideModeBadge={isLiveKind}
          showLoading={showLoading}
          hasShowDetails={Boolean(showDetails)}
          showError={showError}
          currentEpisode={currentEpisode}
          videoRef={videoRef}
          subtitleTracks={subtitleTracks}
          streamQualities={streamQualities}
          selectedQuality={selectedQuality}
          onBack={goBack}
          onQualitySelect={onQualitySelect}
          onRetryStream={retryStream}
          onLoadedMetadata={handleVideoLoadedMetadata}
          onPause={handleVideoPause}
          onEnded={handleVideoEnded}
          onPlaying={handleVideoPlaying}
          onError={handleVideoError}
        />
      </div>
      {isLiveKind ? null : (
        <WatchEpisodeSidebar
          episodes={sidebarEpisodes}
          currentEpisode={currentEpisode}
          currentMode={playbackMode}
          loading={showLoading}
          selecting={loadingEpisode}
          onSelectEpisode={onSidebarSelect}
        />
      )}
    </div>
  );
}
