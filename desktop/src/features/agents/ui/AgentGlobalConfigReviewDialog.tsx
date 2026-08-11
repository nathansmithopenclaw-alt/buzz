import {
  globalConfigReviewRows,
  type AgentManagementGlobalConfigRequest,
} from "../agentManagement";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";

export function AgentGlobalConfigReviewDialog({
  error,
  isPending,
  onApprove,
  onDismiss,
  request,
}: {
  error: string | null;
  isPending: boolean;
  onApprove: () => void;
  onDismiss: () => void;
  request: AgentManagementGlobalConfigRequest | null;
}) {
  const rows = globalConfigReviewRows(request);

  return (
    <AlertDialog open={request !== null}>
      <AlertDialogContent data-testid="agent-global-config-review-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Review global agent defaults</AlertDialogTitle>
          <AlertDialogDescription>
            This request is not covered by the current standing grant. Review
            and save it once, or cancel it. Environment values are hidden;
            effective environment changes may restart affected running agents.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <dl className="space-y-2 text-sm">
          {rows.map(([label, value]) => (
            <div className="grid grid-cols-[9rem_1fr] gap-3" key={label}>
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="break-all">{value}</dd>
            </div>
          ))}
        </dl>
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={isPending} onClick={onDismiss}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction disabled={isPending} onClick={onApprove}>
            {isPending ? "Saving…" : "Save changes"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
