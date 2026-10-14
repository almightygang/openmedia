export type HomeSection = "anime" | "youtube" | "twitch" | "kick" | "other";

export const HOME_SECTIONS: ReadonlyArray<{
  id: HomeSection;
  label: string;
  placeholderTitle: string;
  placeholderDescription: string;
}> = [
  {
    id: "anime",
    label: "Anime",
    placeholderTitle: "Anime",
    placeholderDescription: "Anime home.",
  },
  {
    id: "youtube",
    label: "YouTube",
    placeholderTitle: "YouTube is coming soon",
    placeholderDescription:
      "Nothing functional here yet. Providers for this section land next.",
  },
  {
    id: "twitch",
    label: "Twitch",
    placeholderTitle: "Twitch is coming soon",
    placeholderDescription:
      "Nothing functional here yet. Providers for this section land next.",
  },
  {
    id: "kick",
    label: "Kick",
    placeholderTitle: "Kick is coming soon",
    placeholderDescription:
      "Nothing functional here yet. Providers for this section land next.",
  },
  {
    id: "other",
    label: "Other",
    placeholderTitle: "Other is coming soon",
    placeholderDescription:
      "Nothing functional here yet. Providers for this section land next.",
  },
];

const HOME_SECTION_IDS: ReadonlySet<string> = new Set(HOME_SECTIONS.map((s) => s.id));

export function isHomeSection(value: unknown): value is HomeSection {
  return typeof value === "string" && HOME_SECTION_IDS.has(value);
}

const HOME_SECTION_STORAGE_KEY = "home.section";

export function readStoredHomeSection(): HomeSection {
  try {
    const raw = localStorage.getItem(HOME_SECTION_STORAGE_KEY);
    if (isHomeSection(raw)) return raw;
  } catch {
    // localStorage unavailable (SSR/tests) — fall through to default.
  }
  return "anime";
}

export function storeHomeSection(section: HomeSection): void {
  try {
    localStorage.setItem(HOME_SECTION_STORAGE_KEY, section);
  } catch {
    // Ignore persistence failures; section state still works in-memory.
  }
}
