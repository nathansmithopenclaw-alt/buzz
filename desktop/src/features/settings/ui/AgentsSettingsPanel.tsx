import { AgentDefaultsSettingsCard } from "./AgentDefaultsSettingsCard";
import { ClaudeAccountsSettingsCard } from "./ClaudeAccountsSettingsCard";
import { DeviceRoleSettingsCard } from "./DeviceRoleSettingsCard";
import { HarnessesSettingsPanel } from "./HarnessesSettingsPanel";
import { PreventSleepSettingsCard } from "./PreventSleepSettingsCard";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function AgentsSettingsPanel() {
  return (
    <section className="min-w-0" data-testid="settings-agents">
      <SettingsSectionHeader
        title="Agents"
        description="Control how agents behave in conversations and run on this machine."
      />

      <div className="flex flex-col gap-4">
        <DeviceRoleSettingsCard />
        <ClaudeAccountsSettingsCard />
        <PreventSleepSettingsCard />
        <HarnessesSettingsPanel />
        <AgentDefaultsSettingsCard />
      </div>
    </section>
  );
}
