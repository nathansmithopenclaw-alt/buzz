import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import { AgentCreationPreview } from "./AgentCreationPreview";
import {
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_FIELD_CONTROL_CLASS,
} from "./agentConfigOptions";

export function AgentInstanceAvatarField({
  previewAvatarUrl,
  previewLabel,
  setAvatarUrl,
  setIsAvatarUploadPending,
  isSaving,
  onOpenChange,
  onEditLinkedPersona,
}: {
  previewAvatarUrl: string | null;
  previewLabel: string;
  setAvatarUrl: (value: string) => void;
  setIsAvatarUploadPending: (pending: boolean) => void;
  isSaving: boolean;
  onOpenChange: (open: boolean) => void;
  onEditLinkedPersona?: () => void;
}) {
  return (
    <>
      {/* Avatar is definition-level identity. hideEditControl suppresses
              the internal pencil badge; the CTA below is the only edit path. */}
      <div className="flex flex-col items-center gap-2">
        <AgentCreationPreview
          avatarUrl={previewAvatarUrl}
          hideEditControl
          label={previewLabel}
          onClearAvatar={() => setAvatarUrl("")}
          onUploadPendingChange={setIsAvatarUploadPending}
          onSelectAvatar={setAvatarUrl}
        />
        {onEditLinkedPersona ? (
          <Button
            className="w-full"
            disabled={isSaving}
            onClick={() => {
              onOpenChange(false);
              onEditLinkedPersona();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            Edit avatar
          </Button>
        ) : (
          <p className="text-center text-xs text-muted-foreground">
            Avatar is shared identity
          </p>
        )}
      </div>
    </>
  );
}
export function AgentInstanceNameField({
  name,
  setName,
  isSaving,
}: {
  name: string;
  setName: (value: string) => void;
  isSaving: boolean;
}) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-name"
      >
        Agent name
      </label>
      <div
        className={cn(
          "flex min-h-11 items-center px-3",
          PERSONA_FIELD_SHELL_CLASS,
        )}
      >
        <Input
          autoCorrect="off"
          className={cn("h-8 px-0 py-0 leading-6", PERSONA_FIELD_CONTROL_CLASS)}
          disabled={isSaving}
          id="edit-agent-name"
          onChange={(event) => setName(event.target.value)}
          placeholder="Agent name"
          value={name}
        />
      </div>
    </div>
  );
}
