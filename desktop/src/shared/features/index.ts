export { FeatureGate } from "./FeatureGate";
export { allFeatures, desktopFeatures, getFeature, manifest } from "./manifest";
export {
  FEATURE_ID_ALIASES,
  canonicalizeOverrides,
} from "./featureAliases";
export { getOverrides, setOverride, clearOverride } from "./store";
export type {
  FeatureDefinition,
  FeaturesManifest,
  FeaturePlatform,
} from "./types";
export {
  useFeatureEnabled,
  useFeatureToggle,
  useFeatureSnapshot,
  usePreviewFeatureWarning,
  resolveEnabled,
} from "./useFeatureEnabled";
