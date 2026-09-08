import * as React from "react";

import { useBakedBuildEnvQuery } from "../hooks";
import { useGlobalAgentConfig } from "../useGlobalAgentConfig";
import { BUZZ_AGENT_THINKING_EFFORT } from "./buzzAgentConfig";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers";

// Keep omitted overrides stable so downstream discovery effects retain in-flight work.
const EMPTY_INHERITED_ENV: Record<string, string> = Object.freeze({});

export function useAgentDialogDefaults({
  inheritedEnvVars = EMPTY_INHERITED_ENV,
  open,
  runtimeId,
}: {
  inheritedEnvVars?: Record<string, string>;
  open: boolean;
  runtimeId: string;
}) {
  const { globalConfig } = useGlobalAgentConfig();
  const { data: bakedEnv } = useBakedBuildEnvQuery({ enabled: open });
  const inheritedDefaults = getInheritedAgentDefaults(
    globalConfig,
    bakedEnv,
    runtimeId,
  );
  const effectiveInheritedEnvVars = React.useMemo(
    () => ({
      ...globalConfig.env_vars,
      ...inheritedEnvVars,
      ...(inheritedDefaults.effort.value
        ? { [BUZZ_AGENT_THINKING_EFFORT]: inheritedDefaults.effort.value }
        : {}),
    }),
    [globalConfig.env_vars, inheritedDefaults.effort.value, inheritedEnvVars],
  );
  return {
    globalConfig,
    inheritedDefaults,
    inheritedEnvVars: effectiveInheritedEnvVars,
  };
}
