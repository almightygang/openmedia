/**
 * Toggle stream providers without code changes.
 * AnimeParadise is anime VOD; Twitch (anonymous GQL) and YouTube (anonymous
 * InnerTube) are live/video. Kick is intentionally skipped — its home section
 * stays "coming soon".
 */
export const streamProviderAvailability = {
  animeparadise: true,
  twitch: true,
  youtube: true,
} as const;
