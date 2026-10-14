/**
 * Toggle stream providers without code changes.
 * Only AnimeParadise resolves playable streams in this build; the other
 * upstreams were removed (see README). This stays as a map so the Settings
 * provider list keeps working unchanged.
 */
export const streamProviderAvailability = {
  animeparadise: true,
} as const;
