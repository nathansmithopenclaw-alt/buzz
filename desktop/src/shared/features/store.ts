/**
 * Persistence layer for feature flag overrides.
 *
 * The localStorage key is derived from `manifest.version` so a schema bump
 * naturally orphans the old key. Retired ids are mapped in FEATURE_ID_ALIASES.
 *
 *   buzz-feature-overrides-v${manifest.version}
 *     → JSON object of { [featureId]: boolean }
 */
import { canonicalizeOverrides, type FeatureOverrides } from "./featureAliases";
import { manifest } from "./manifest";

export const OVERRIDES_KEY = `buzz-feature-overrides-v${manifest.version}`;
export type { FeatureOverrides };
export { FEATURE_ID_ALIASES, canonicalizeOverrides } from "./featureAliases";

/** Read all user overrides from localStorage */
export function getOverrides(): FeatureOverrides {
  try {
    const raw = window.localStorage.getItem(OVERRIDES_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
      return {};
    const featureIds = new Set(manifest.features.map((feature) => feature.id));
    return canonicalizeOverrides(parsed as Record<string, unknown>, featureIds);
  } catch {
    return {};
  }
}

/** Persist a single feature override */
export function setOverride(featureId: string, enabled: boolean): void {
  const overrides = getOverrides();
  overrides[featureId] = enabled;
  window.localStorage.setItem(OVERRIDES_KEY, JSON.stringify(overrides));
}

/** Remove a single feature override (revert to default) */
export function clearOverride(featureId: string): void {
  const overrides = getOverrides();
  delete overrides[featureId];
  window.localStorage.setItem(OVERRIDES_KEY, JSON.stringify(overrides));
}
