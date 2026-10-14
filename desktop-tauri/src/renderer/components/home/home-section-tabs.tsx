import { Clapperboard, Gamepad2, LayoutGrid, Twitch, Youtube } from "lucide-react";
import React from "react";

import { Tabs, TabsList, TabsTrigger } from "@/renderer/components/ui/tabs";
import { cn } from "@/renderer/lib/utils";
import { HOME_SECTIONS, type HomeSection } from "@/shared/home-sections";

import { useHomeSection } from "./home-section-context";

export const HOME_SECTION_ICONS = {
  anime: Clapperboard,
  youtube: Youtube,
  twitch: Twitch,
  kick: Gamepad2,
  other: LayoutGrid,
} as const;

/**
 * Compact section switcher that lives in the titlebar next to the search box.
 * Small enough for the fixed h-12 drag bar; labels collapse on narrow widths.
 * `shrink-0` + margin keep it clear of the search input; `clickable` opts out
 * of the titlebar drag region (alongside Tauri's `data-tauri-drag-region`,
 * which already leaves native buttons/inputs interactive).
 */
export function HomeSectionTitlebarTabs({ className }: { className?: string }) {
  const { section, setSection } = useHomeSection();

  return (
    <Tabs
      value={section}
      onValueChange={(value) => setSection(value as HomeSection)}
      className={cn("clickable hidden shrink-0 sm:block ml-2 xl:ml-16", className)}
    >
      <TabsList aria-label="Home sections" className="h-8 gap-0.5 px-1">
        {HOME_SECTIONS.map((entry) => {
          const Icon = HOME_SECTION_ICONS[entry.id];
          return (
            <TabsTrigger key={entry.id} value={entry.id} className="h-6 gap-1 px-2 text-xs">
              <Icon className="h-3.5 w-3.5" aria-hidden="true" />
              <span className="hidden xl:inline">{entry.label}</span>
              <span className="sr-only">{entry.label}</span>
            </TabsTrigger>
          );
        })}
      </TabsList>
    </Tabs>
  );
}
