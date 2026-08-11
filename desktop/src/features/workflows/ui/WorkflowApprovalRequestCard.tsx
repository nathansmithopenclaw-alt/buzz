import { Check, Clock, X } from "lucide-react";
import * as React from "react";

import {
  useApprovalMutation,
  useApprovalResolutionQuery,
} from "@/features/workflows/hooks";
import { parseWorkflowApprovalRequest } from "@/features/workflows/lib/workflowApprovalRequest";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Textarea } from "@/shared/ui/textarea";

type ApprovalOutcome = "approved" | "rejected" | null;

export function WorkflowApprovalRequestCard({
  content,
  tags,
}: {
  content: string;
  tags?: string[][];
}) {
  const identityQuery = useIdentityQuery();
  const approvalMutation = useApprovalMutation();
  const [note, setNote] = React.useState("");
  const [outcome, setOutcome] = React.useState<ApprovalOutcome>(null);
  const request = React.useMemo(
    () => parseWorkflowApprovalRequest(tags),
    [tags],
  );
  const currentPubkey = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : null;
  const isApprover = Boolean(
    currentPubkey &&
      request.approverPubkey &&
      currentPubkey === normalizePubkey(request.approverPubkey),
  );
  const isExpired = Boolean(
    request.expiresAt && request.expiresAt.getTime() <= Date.now(),
  );
  const canResolvePersistedDecision = Boolean(
    isApprover && request.expiresAt && !isExpired,
  );
  const approvalResolutionQuery = useApprovalResolutionQuery(
    canResolvePersistedDecision ? request.tokenHash : null,
    canResolvePersistedDecision ? request.approverPubkey : null,
  );
  const persistedOutcome: ApprovalOutcome = approvalResolutionQuery.data?.status
    ? approvalResolutionQuery.data.status === "granted"
      ? "approved"
      : "rejected"
    : null;
  const displayedOutcome = outcome ?? persistedOutcome;
  const isActionable = Boolean(
    request.token &&
      request.tokenHash &&
      request.expiresAt &&
      isApprover &&
      !isExpired &&
      !displayedOutcome &&
      !approvalResolutionQuery.isPending,
  );
  const errorMessage = approvalMutation.error
    ? approvalMutation.error instanceof Error
      ? approvalMutation.error.message
      : "The approval decision could not be submitted."
    : null;

  const decide = (action: "grant" | "deny") => {
    if (!request.token) return;
    approvalMutation.mutate(
      {
        token: request.token,
        action,
        note: note.trim() || undefined,
      },
      {
        onSuccess: () => {
          setOutcome(action === "grant" ? "approved" : "rejected");
        },
      },
    );
  };

  return (
    <div
      className="max-w-2xl rounded-xl border border-amber-500/35 bg-amber-500/[0.06] p-3"
      data-testid="workflow-approval-request-card"
    >
      <div className="mb-2 flex items-center gap-2 text-sm font-semibold">
        <Clock className="h-4 w-4 text-amber-500" />
        Approval required
      </div>
      <Markdown className="text-sm" content={content} />

      {request.expiresAt ? (
        <p className="mt-2 text-xs text-muted-foreground">
          Expires {request.expiresAt.toLocaleString()}
        </p>
      ) : null}

      {displayedOutcome ? (
        <div
          className={
            displayedOutcome === "approved"
              ? "mt-3 flex items-center gap-2 text-sm font-medium text-green-600"
              : "mt-3 flex items-center gap-2 text-sm font-medium text-destructive"
          }
          data-testid="workflow-approval-outcome"
        >
          {displayedOutcome === "approved" ? (
            <Check className="h-4 w-4" />
          ) : (
            <X className="h-4 w-4" />
          )}
          {displayedOutcome === "approved" ? "Approved" : "Rejected"}
        </div>
      ) : !request.expiresAt ? (
        <p className="mt-3 text-sm font-medium text-destructive">
          This approval request has an invalid expiration.
        </p>
      ) : isExpired ? (
        <p className="mt-3 text-sm font-medium text-muted-foreground">
          This approval request has expired.
        </p>
      ) : !request.token || !request.tokenHash ? (
        <p className="mt-3 text-sm font-medium text-destructive">
          This approval request is missing its decision token.
        </p>
      ) : !isApprover ? (
        <p className="mt-3 text-xs text-muted-foreground">
          Only the designated approver can act on this request.
        </p>
      ) : (
        <>
          <Textarea
            aria-label="Approval note"
            className="mt-3 h-16 resize-none text-xs"
            disabled={approvalMutation.isPending}
            onChange={(event) => setNote(event.target.value)}
            placeholder="Optional note"
            value={note}
          />
          <div className="mt-2 flex gap-2">
            <Button
              className="bg-green-600 text-white hover:bg-green-700"
              disabled={!isActionable || approvalMutation.isPending}
              onClick={() => decide("grant")}
              size="sm"
              type="button"
            >
              <Check className="mr-1 h-4 w-4" />
              Approve
            </Button>
            <Button
              disabled={!isActionable || approvalMutation.isPending}
              onClick={() => decide("deny")}
              size="sm"
              type="button"
              variant="destructive"
            >
              <X className="mr-1 h-4 w-4" />
              Reject
            </Button>
          </div>
          {errorMessage ? (
            <p className="mt-2 text-xs text-destructive" role="alert">
              {errorMessage}
            </p>
          ) : null}
        </>
      )}
    </div>
  );
}
