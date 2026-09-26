import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import { SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";
import { Button } from "@/shared/ui/button";
import {
  downloadDictationModel,
  listSpeechModels,
  removeSpeechModel,
} from "../api";

const size = (bytes: number) => `${Math.ceil(bytes / 1e6)} MB`;
const gb = (bytes: number) => `${(bytes / 1e9).toFixed(2)} GB`;

export function DictationSpeechRow({
  modelId,
  disabled,
  onChange,
}: {
  modelId: string;
  disabled: boolean;
  onChange: (modelId: string) => void;
}) {
  const [busy, setBusy] = React.useState(false);
  const query = useQuery({
    queryKey: ["dictation-speech-catalog"],
    queryFn: listSpeechModels,
    refetchInterval: (query) =>
      query.state.data?.some(
        (model) =>
          typeof model.status === "object" && "downloading" in model.status,
      )
        ? 1500
        : false,
  });
  const model = query.data?.find((model) => model.id === modelId);
  const downloading =
    model && typeof model.status === "object" && "downloading" in model.status
      ? model.status.downloading.progress_percent
      : null;
  const mutate = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
      await query.refetch();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SettingsOptionRow data-testid="dictation-model-row">
      <label className="font-medium" htmlFor="dictation-speech-model">
        Speech model
      </label>
      <div className="w-80 max-w-full space-y-2">
        <AgentDropdownSelect
          id="dictation-speech-model"
          disabled={disabled || busy}
          value={modelId}
          selectedLabel={model?.name}
          onValueChange={onChange}
          options={(query.data ?? []).map((model) => ({
            value: model.id,
            label: (
              <span className="block whitespace-normal">
                <span className="block">{model.name}</span>
                <span className="block text-xs text-black/60">
                  {model.language} · {size(model.sizeBytes)} on disk
                </span>
                {model.recommended ? (
                  <span className="inline-block rounded bg-black/5 px-1.5 text-xs text-black/70">
                    Recommended
                  </span>
                ) : null}
              </span>
            ),
          }))}
        />
        {query.error ? (
          <p role="alert" className="text-xs text-destructive">
            Could not load speech models.{" "}
            <button
              type="button"
              className="underline"
              onClick={() => void query.refetch()}
            >
              Retry
            </button>
          </p>
        ) : null}
        {model ? (
          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{size(model.sizeBytes)} on disk</span>
            {downloading !== null ? (
              <span role="status">Downloading… {downloading}%</span>
            ) : model.status === "ready" ? (
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void mutate(() => removeSpeechModel(model.id))}
              >
                Remove
              </Button>
            ) : (
              <Button
                size="sm"
                disabled={busy || !model.enoughSpace}
                onClick={() =>
                  void mutate(() => downloadDictationModel(model.id))
                }
              >
                Download
              </Button>
            )}
            {model.status !== "ready" &&
            downloading === null &&
            !model.enoughSpace ? (
              <p role="alert" className="text-destructive">
                {model.freeBytes === null
                  ? "Cannot check available disk space"
                  : `Not enough space · ${gb(model.requiredBytes)} required · ${gb(model.freeBytes)} available`}
                <button
                  type="button"
                  className="ml-1 underline"
                  onClick={() => void query.refetch()}
                >
                  Check again
                </button>
              </p>
            ) : null}
            {typeof model.status === "object" && "error" in model.status ? (
              <p role="alert" className="text-destructive">
                {model.status.error}
              </p>
            ) : null}
          </div>
        ) : null}
      </div>
    </SettingsOptionRow>
  );
}
