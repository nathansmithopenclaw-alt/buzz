import type { AgentTeamManagementRequest } from "../agentTeamManagement";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type AgentTeamManagementReviewDialogProps = {
  error: string | null;
  isPending: boolean;
  request: AgentTeamManagementRequest | null;
  onApprove: () => void;
  onDismiss: () => void;
};

function requestSummary(request: AgentTeamManagementRequest): string {
  if (request.action === "create") {
    return `Create team “${request.request.name}” with ${request.request.members.length} member(s).`;
  }
  if (request.action === "update") {
    const changes = [
      request.request.name !== undefined ? "name" : null,
      request.request.description !== undefined ? "description" : null,
      request.request.instructions !== undefined ? "instructions" : null,
      request.request.members !== undefined ? "members" : null,
    ].filter((value): value is string => value !== null);
    return `Update team “${request.request.team}” (${changes.join(", ") || "no field changes"}).`;
  }
  return `Deploy team “${request.request.team}” to channel ${request.request.channelId}.`;
}

/** Owner review surface used when a standing teams grant is absent or revoked. */
export function AgentTeamManagementReviewDialog({
  error,
  isPending,
  request,
  onApprove,
  onDismiss,
}: AgentTeamManagementReviewDialogProps) {
  return (
    <Dialog open={request !== null}>
      <DialogContent
        onEscapeKeyDown={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>Review team request</DialogTitle>
          <DialogDescription>
            {request
              ? requestSummary(request)
              : "Review this team-management request."}
          </DialogDescription>
        </DialogHeader>
        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          <Button
            disabled={isPending}
            onClick={onDismiss}
            type="button"
            variant="outline"
          >
            Dismiss
          </Button>
          <Button disabled={isPending} onClick={onApprove} type="button">
            {isPending ? "Applying…" : "Approve and apply"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
