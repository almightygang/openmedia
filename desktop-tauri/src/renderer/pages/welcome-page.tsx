import React, { useCallback } from "react";
import { useNavigate } from "react-router-dom";

import { HomeAnimeSection } from "@/renderer/components/home/home-anime-section";
import { HomePlaceholderSection } from "@/renderer/components/home/home-placeholder-section";
import { HOME_SECTION_ICONS } from "@/renderer/components/home/home-section-tabs";
import { useHomeSection } from "@/renderer/components/home/home-section-context";
import { PinnedIssuesBanner } from "@/renderer/components/pinned-issues-banner";
import { usePinnedGithubIssues } from "@/renderer/hooks/use-pinned-github-issues";
import { HOME_SECTIONS } from "@/shared/home-sections";
import type { Episode } from "@/shared/types";

export function WelcomePage() {
  const navigate = useNavigate();
  const { section } = useHomeSection();
  const { issues: pinnedIssues } = usePinnedGithubIssues();
  const showPinnedBanner = pinnedIssues.length > 0;

  const openRecentAnime = useCallback(
    (episode: Episode) => {
      navigate("/watch", { state: { episode } });
    },
    [navigate]
  );

  const activeMeta = HOME_SECTIONS.find((s) => s.id === section) ?? HOME_SECTIONS[0];

  const sectionProvider =
    section === "twitch" ? "twitch" as const : section === "youtube" ? "youtube" as const : null;

  return (
    <div className="w-full max-w-[1600px] mx-auto flex flex-col gap-6 p-6 md:p-8">
      <div className="-mt-6 -mx-6 md:-mt-8 md:-mx-8">
        {showPinnedBanner ? <PinnedIssuesBanner issues={pinnedIssues} /> : null}
      </div>

      {section === "anime" ? (
        <HomeAnimeSection onOpenEpisode={openRecentAnime} />
      ) : sectionProvider ? (
        <HomeAnimeSection
          key={sectionProvider}
          provider={sectionProvider}
          onOpenEpisode={openRecentAnime}
        />
      ) : (
        <HomePlaceholderSection
          icon={HOME_SECTION_ICONS[activeMeta.id]}
          title={activeMeta.placeholderTitle}
          description={activeMeta.placeholderDescription}
        />
      )}
    </div>
  );
}
