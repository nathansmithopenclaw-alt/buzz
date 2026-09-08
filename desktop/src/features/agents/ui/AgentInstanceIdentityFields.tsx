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
  avatarSyncPending,
  setAvatarUrl,
  setIsAvatarUploadPending,
  isSaving,
  onOpenChange,
  onEditLinkedPersona,
}: {
  previewAvatarUrl: string | null;
  previewLabel: string;
  avatarSyncPending?: boolean;
  setAvatarUrl: (value: string) => void;
  setIsAvatarUploadPending: (pending: boolean) => void;
  isSaving: boolean;
  onOpenChange: (open: boolean) => void;
  onEditLinkedPersona?: () => void;
}) {
  return (
    <>
      {/* The picture editor changes this instance; the secondary action edits the shared definition. */}
      <div className="flex flex-col items-center gap-2">
        <AgentCreationPreview
          avatarUrl={previewAvatarUrl}
          label={previewLabel}
          onClearAvatar={() => setAvatarUrl("")}
          onUploadPendingChange={setIsAvatarUploadPending}
          onSelectAvatar={setAvatarUrl}
        />
        {avatarSyncPending && (
          <p
            role="status"
            className="text-center text-xs text-muted-foreground"
          >
            Picture saved. Publication pending; Buzz will retry automatically.
          </p>
        )}
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
            Edit shared definition
          </Button>
        ) : (
          <p className="text-center text-xs text-muted-foreground">
            Picture changes apply to this agent
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
