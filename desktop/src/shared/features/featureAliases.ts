export type FeatureOverrides = Record<string, boolean>;

/** Retired experiment ids that still appear in persisted overrides. */
export const FEATURE_ID_ALIASES: Record<string, string> = {
  fleetVisibility: "remoteControl",
};

export function canonicalizeOverrides(
  parsed: Record<string, unknown>,
  featureIds: ReadonlySet<string>,
): FeatureOverrides {
  const canonical: FeatureOverrides = {};
  for (const [key, value] of Object.entries(parsed)) {
    if (typeof value !== "boolean") continue;
    const aliased = FEATURE_ID_ALIASES[key];
    if (
      aliased &&
      featureIds.has(aliased) &&
      canonical[aliased] === undefined
    ) {
      canonical[aliased] = value;
    }
  }
  for (const [key, value] of Object.entries(parsed)) {
    if (typeof value !== "boolean" || !featureIds.has(key)) continue;
    canonical[key] = value;
  }
  return canonical;
}
