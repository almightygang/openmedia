import React from "react";

import { EpisodeCard } from "@/renderer/components/episode-card";
import { HomeHeroCarousel } from "@/renderer/components/home-hero-carousel";
import { HorizontalCarousel, HorizontalCarouselCard } from "@/renderer/components/horizontal-carousel";
import { Badge } from "@/renderer/components/ui/badge";
import { Skeleton } from "@/renderer/components/ui/skeleton";
import { useResumeHistoryEntry } from "@/renderer/hooks/use-resume-history-entry";
import { useWelcomeRecentlyUploaded } from "@/renderer/hooks/use-welcome-recent-uploads";
import { useWelcomeRecentlyWatched } from "@/renderer/hooks/use-welcome-recently-watched";
import type { Episode, StreamProvider } from "@/shared/types";

const RECENT_PAGE_SIZE = 12;

const SECTION_COPY: Record<
  StreamProvider,
  { uploads: string; emptyUploads: string; watching: string; emptyWatching: string; hero: boolean }
> = {
  animeparadise: {
    uploads: "Recently uploaded",
    emptyUploads: "No recent anime found.",
    watching: "Continue watching",
    emptyWatching: "No recently watched anime.",
    hero: true,
  },
  twitch: {
    uploads: "Live now",
    emptyUploads: "No live Twitch channels found.",
    watching: "Continue watching",
    emptyWatching: "No recently watched Twitch streams.",
    hero: false,
  },
  youtube: {
    uploads: "Latest videos",
    emptyUploads: "No recent YouTube videos found.",
    watching: "Continue watching",
    emptyWatching: "No recently watched YouTube videos.",
    hero: false,
  },
};

function formatBadgeViews(views: number, isLive?: boolean): string {
  const short = views >= 1000 ? `${(views / 1000).toFixed(1).replace(/\.0$/, "")}K` : views;
  return `${short}${isLive ? " watching" : " views"}`;
}

function RecentUploadsCarouselSkeleton({ count = 8 }: { count?: number }) {
  return (
    <div
      className="flex gap-3 overflow-hidden pb-2 px-8 -mx-8"
      aria-busy="true"
      aria-label="Loading recently uploaded"
    >
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="flex-shrink-0 w-36 sm:w-40">
          <Skeleton className="w-full aspect-[2/3] rounded-2xl" />
        </div>
      ))}
    </div>
  );
}

export function HomeAnimeSection({
  onOpenEpisode,
  provider = "animeparadise",
}: {
  onOpenEpisode: (episode: Episode) => void;
  provider?: StreamProvider;
}) {
  const copy = SECTION_COPY[provider] ?? SECTION_COPY.animeparadise;
  const { resumeHistoryEntry, disabledProviderDialog } = useResumeHistoryEntry();
  const { recentUploads, recentUploadsLoading } = useWelcomeRecentlyUploaded(
    RECENT_PAGE_SIZE,
    provider
  );
  const { recentlyWatched, recentlyWatchedLoading } = useWelcomeRecentlyWatched(provider);
  const isAnime = provider === "animeparadise";
  // Live/video tabs show last-played (compact); anime keeps full continue-watching.
  const visibleHistory = isAnime ? recentlyWatched : recentlyWatched.slice(0, 6);

  return (
    <>
      {disabledProviderDialog}
      {copy.hero ? (
        <div className="-mt-6 -mx-6 md:-mt-8 md:-mx-8">
          <HomeHeroCarousel />
        </div>
      ) : null}
      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-semibold">{copy.watching}</h2>
        {recentlyWatchedLoading ? (
          <p className="text-muted-foreground text-sm">Loading…</p>
        ) : visibleHistory.length > 0 ? (
          <HorizontalCarousel>
            {visibleHistory.map((entry) => (
              <EpisodeCard
                key={entry.id}
                layout="carousel"
                orientation={isAnime ? "portrait" : "landscape"}
                thumbnailUrl={entry.episode.thumbnail}
                badge={
                  isAnime
                    ? `Episode ${String(entry.episode.index)}`
                    : (() => {
                        const views = entry.episode.views ?? entry.episode.viewerCount ?? null;
                        return views != null
                          ? formatBadgeViews(views, entry.episode.isLive)
                          : (entry.episode.isLive ? "LIVE" : "Video");
                      })()
                }
                subtitle={
                  entry.episode.title.english ??
                  entry.episode.title.romanji ??
                  entry.episode.title.native ??
                  entry.episode.id
                }
                onClick={() => {
                  resumeHistoryEntry(entry);
                }}
                totalDurationMs={isAnime || !entry.episode.isLive ? entry.totalDurationMs : undefined}
                currentDurationMs={isAnime || !entry.episode.isLive ? entry.currentDurationMs : undefined}
              />
            ))}
          </HorizontalCarousel>
        ) : (
          <p className="text-muted-foreground text-sm">{copy.emptyWatching}</p>
        )}
      </section>
      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-semibold">{copy.uploads}</h2>
        {recentUploadsLoading ? (
          <RecentUploadsCarouselSkeleton />
        ) : recentUploads.length > 0 ? (
          <HorizontalCarousel>
            {recentUploads.map((episode) => {
              const live = episode.provider === "twitch" || episode.provider === "youtube";
              const views = episode.views ?? episode.viewerCount ?? null;
              const badge = live
                ? views != null
                  ? formatBadgeViews(views, episode.isLive)
                  : (episode.isLive ? "LIVE" : "Video")
                : `Episode ${episode.index}`;
              const title =
                episode.title.english ??
                episode.title.romanji ??
                episode.title.native ??
                episode.providerId;
              const key = `${episode.id || episode.providerId}-${episode.index}-${episode.mode}`;
              // Live/video rows render as 16:9 landscape tiles (native stream
              // thumbnails); anime keeps portrait poster cards.
              return live ? (
                <EpisodeCard
                  key={key}
                  layout="carousel"
                  orientation="landscape"
                  thumbnailUrl={episode.thumbnail}
                  badge={badge}
                  subtitle={title}
                  onClick={() => onOpenEpisode(episode)}
                />
              ) : (
                <HorizontalCarouselCard
                  key={key}
                  item={{
                    id: key,
                    coverUrl: episode.thumbnail,
                    title,
                    badges: (
                      <Badge
                        variant="glass"
                        className="text-white flex items-center gap-1 align-middle"
                      >
                        {badge}
                      </Badge>
                    ),
                    onClick: () => onOpenEpisode(episode),
                  }}
                />
              );
            })}
          </HorizontalCarousel>
        ) : (
          <p className="text-muted-foreground text-sm">{copy.emptyUploads}</p>
        )}
      </section>
    </>
  );
}
