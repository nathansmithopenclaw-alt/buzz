import * as React from "react";

import {
  getFleetVisibilitySettings,
  setFleetVisibilityEnabled,
  setFleetVisibilityRole,
  type FleetVisibilityRole,
  type FleetVisibilitySettings,
} from "@/shared/api/tauriManagedAgents";

const CHANGE_EVENT = "buzz:fleet-visibility-settings-changed";

function notifyChange() {
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

export async function updateFleetVisibilityEnabled(enabled: boolean) {
  const settings = await setFleetVisibilityEnabled(enabled);
  notifyChange();
  return settings;
}

export async function updateFleetVisibilityRole(role: FleetVisibilityRole) {
  const settings = await setFleetVisibilityRole(role);
  notifyChange();
  return settings;
}

export function useFleetVisibilitySettings(): FleetVisibilitySettings | null {
  const [settings, setSettings] =
    React.useState<FleetVisibilitySettings | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void getFleetVisibilitySettings()
        .then((next) => {
          if (!cancelled) setSettings(next);
        })
        .catch((error) => {
          console.warn("[FleetVisibility] settings unavailable:", error);
        });
    };
    refresh();
    window.addEventListener(CHANGE_EVENT, refresh);
    return () => {
      cancelled = true;
      window.removeEventListener(CHANGE_EVENT, refresh);
    };
  }, []);

  return settings;
}
