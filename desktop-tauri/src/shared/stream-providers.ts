import { streamProviderAvailability } from "./stream-providers.config";

export type StreamProviderName = keyof typeof streamProviderAvailability;

export const STREAM_PROVIDER_LABELS: Record<StreamProviderName, string> = {
  animeparadise: "AnimeParadise",
  twitch: "Twitch",
  youtube: "YouTube",
};

export const ALL_STREAM_PROVIDER_NAMES = Object.keys(
  streamProviderAvailability
) as StreamProviderName[];

export function isStreamProviderName(value: unknown): value is StreamProviderName {
  return typeof value === "string" && value in streamProviderAvailability;
}

export function getEnabledStreamProviders(): StreamProviderName[] {
  return ALL_STREAM_PROVIDER_NAMES.filter((name) => streamProviderAvailability[name]);
}

export function isStreamProviderEnabled(name: StreamProviderName): boolean {
  return streamProviderAvailability[name];
}

export function isHistoryProviderDisabled(provider: unknown): provider is StreamProviderName {
  return isStreamProviderName(provider) && !isStreamProviderEnabled(provider);
}

export function resolveEnabledStreamProvider(value: unknown): StreamProviderName {
  const enabled = getEnabledStreamProviders();
  if (enabled.length === 0) {
    throw new Error("No stream providers are enabled in stream-providers.config.ts");
  }

  if (isStreamProviderName(value) && isStreamProviderEnabled(value)) {
    return value;
  }

  return enabled[0];
}

/**
 * Normalize a stored enabled-provider list (multi-provider mode).
 * Accepts arrays, single names, nullish values; drops unknown/disabled
 * names and falls back to the first available provider.
 */
export function normalizeEnabledStreamProviders(value: unknown): StreamProviderName[] {
  const available = getEnabledStreamProviders();
  const fallback = available.length > 0 ? [available[0]] : [];
  const candidates: unknown[] = Array.isArray(value) ? value : value == null ? [] : [value];
  const out: StreamProviderName[] = [];
  for (const candidate of candidates) {
    if (isStreamProviderName(candidate) && isStreamProviderEnabled(candidate)) {
      if (!out.includes(candidate)) out.push(candidate);
    }
  }
  return out.length > 0 ? out : fallback;
}
