import { REMOTE_CONTROL_FEATURE_ID } from "@/features/agents/lib/remoteControlFeature";
import {
  updateFleetVisibilityRole,
  useFleetVisibilitySettings,
} from "@/features/agents/lib/useFleetVisibilitySettings";
import { useFeatureEnabled } from "@/shared/features";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function DeviceRoleSettingsCard() {
  const experimentEnabled = useFeatureEnabled(REMOTE_CONTROL_FEATURE_ID);
  const settings = useFleetVisibilitySettings();
  if (!experimentEnabled || !settings?.enabled) return null;

  return (
    <SettingsOptionGroup title="This Mac">
      <SettingsOptionRow>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">Device role</p>
          <p className="text-xs text-muted-foreground/70" data-settings-subcopy>
            Hosts run agent processes on this Mac. Observers view and message
            the fleet without running agents locally.
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
                  console.error("Failed to update device role:", error);
                });
              }}
              type="button"
            >
              {role === "host" ? "Host" : "Observer"}
            </button>
          ))}
        </div>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
