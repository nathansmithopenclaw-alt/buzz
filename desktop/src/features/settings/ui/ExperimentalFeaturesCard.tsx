import { setAgentManagedProfiles } from "@/shared/api/tauri";
import {
  updateFleetVisibilityEnabled,
  updateFleetVisibilityRole,
  useFleetVisibilitySettings,
} from "@/features/agents/lib/useFleetVisibilitySettings";
import { desktopFeatures, useFeatureToggle } from "@/shared/features";
import type { FeatureDefinition } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

function FeatureRow({ feature }: { feature: FeatureDefinition }) {
  const [enabled, toggle] = useFeatureToggle(feature.id);
  const switchId = `feature-toggle-${feature.id}`;

  return (
    <SettingsOptionRow>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium" id={`${switchId}-label`}>
          {feature.name}
        </p>
        <p className="text-xs text-muted-foreground/70" data-settings-subcopy>
          {feature.description}
        </p>
      </div>
      <Switch
        aria-labelledby={`${switchId}-label`}
        checked={enabled}
        data-testid={switchId}
        onCheckedChange={(value) => {
          toggle(value);
          if (feature.id === "agentManagedProfiles") {
            void setAgentManagedProfiles(value).catch((error) => {
              console.error(
                "Failed to apply agent-managed profiles setting:",
                error,
              );
            });
          }
          if (feature.id === "fleetVisibility") {
            void updateFleetVisibilityEnabled(value).catch((error) => {
              toggle(!value);
              console.error("Failed to apply Fleet Visibility setting:", error);
            });
          }
        }}
      />
    </SettingsOptionRow>
  );
}

function FleetVisibilityRoleRow() {
  const settings = useFleetVisibilitySettings();
  if (!settings?.enabled) return null;

  return (
    <SettingsOptionRow>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">Device role</p>
        <p className="text-xs text-muted-foreground/70" data-settings-subcopy>
          Hosts run and configure agents. Observers see and message the fleet
          without running agents locally.
        </p>
      </div>
      <div className="flex rounded-lg border border-border/70 p-0.5">
        {(["host", "observer"] as const).map((role) => (
          <button
            aria-pressed={settings.role === role}
            className={
              settings.role === role
                ? "rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground"
                : "rounded-md px-3 py-1.5 text-xs font-medium text-muted-foreground hover:bg-muted"
            }
            data-testid={`fleet-visibility-role-${role}`}
            key={role}
            onClick={() => {
              void updateFleetVisibilityRole(role).catch((error) => {
                console.error("Failed to update Fleet Visibility role:", error);
              });
            }}
            type="button"
          >
            {role === "host" ? "Host" : "Observer"}
          </button>
        ))}
      </div>
    </SettingsOptionRow>
  );
}

export function ExperimentalFeaturesCard() {
  // Manifest is preview-only by definition; every desktop entry is a preview
  // feature.
  const previewFeatures = desktopFeatures;

  return (
    <section className="min-w-0" data-testid="settings-experimental">
      <SettingsSectionHeader
        title="Experiments"
        description={
          <>
            These features are functional but still being refined. Enable them
            to try new capabilities early.
          </>
        }
      />

      <SettingsOptionGroup title="Features">
        {previewFeatures.map((f) => (
          <FeatureRow feature={f} key={f.id} />
        ))}
        <FleetVisibilityRoleRow />
      </SettingsOptionGroup>
    </section>
  );
}
