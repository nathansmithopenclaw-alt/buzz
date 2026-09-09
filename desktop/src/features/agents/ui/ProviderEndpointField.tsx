import { SaveLegacyConnectionButton } from "@/features/model-connections/SaveLegacyConnectionButton";
import { ModelConnectionPicker } from "@/features/model-connections/ModelConnectionPicker";
import { CONNECTION_ENV } from "@/features/model-connections/api";
import { Input } from "@/shared/ui/input";
import { useId } from "react";

/** Endpoint and optional token use the same saved env as discovery and launch. */
export function ProviderEndpointField({
  provider,
  envVars,
  inheritedEnvVars = {},
  onChange,
  disabled = false,
}: {
  provider: string;
  envVars: Record<string, string>;
  inheritedEnvVars?: Record<string, string>;
  onChange: (
    env: Record<string, string>,
    endpointChanged: boolean,
    provider?: string,
  ) => void;
  disabled?: boolean;
}) {
  const id = useId();
  if (provider === "buzz-local")
    return (
      <ModelConnectionPicker
        value={
          envVars[CONNECTION_ENV] ?? inheritedEnvVars[CONNECTION_ENV] ?? ""
        }
        disabled={disabled}
        onChange={(id) => onChange({ ...envVars, [CONNECTION_ENV]: id }, true)}
      />
    );
  if (provider !== "deepseek-compatible") return null;
  return (
    <div className="space-y-3">
      <SaveLegacyConnectionButton
        url={
          envVars.DSH_LLM_BASE_URL ?? inheritedEnvVars.DSH_LLM_BASE_URL ?? ""
        }
        apiKey={envVars.DSH_API_KEY ?? inheritedEnvVars.DSH_API_KEY}
        disabled={disabled}
        onSaved={(id) => {
          const next: Record<string, string> = {
            ...envVars,
            [CONNECTION_ENV]: id,
          };
          delete next.DSH_API_KEY;
          delete next.DSH_LLM_BASE_URL;
          delete next.DSH_PROVIDER;
          onChange(next, false, "buzz-local");
        }}
      />
      <div className="space-y-1.5">
        <label htmlFor={`${id}-url`}>Model endpoint URL</label>
        <Input
          id={`${id}-url`}
          type="url"
          disabled={disabled}
          autoComplete="off"
          placeholder={
            inheritedEnvVars.DSH_LLM_BASE_URL || "http://host:port/v1"
          }
          value={envVars.DSH_LLM_BASE_URL ?? ""}
          onChange={(event) =>
            onChange({ ...envVars, DSH_LLM_BASE_URL: event.target.value }, true)
          }
        />
        <p className="text-xs text-muted-foreground">
          Models are loaded from this endpoint. Include its API base path, such
          as /v1.
        </p>
      </div>
      <div className="space-y-1.5">
        <label htmlFor={`${id}-key`}>Endpoint API key (optional)</label>
        <Input
          id={`${id}-key`}
          type="password"
          disabled={disabled}
          autoComplete="off"
          placeholder={
            inheritedEnvVars.DSH_API_KEY
              ? "Inherited API key"
              : "Leave blank if this endpoint needs no key"
          }
          value={envVars.DSH_API_KEY ?? ""}
          onChange={(event) =>
            onChange({ ...envVars, DSH_API_KEY: event.target.value }, false)
          }
        />
      </div>
    </div>
  );
}
