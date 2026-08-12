import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  publishDispatchComment,
  publishDispatchIssueMutation,
} from "../mutations";
import {
  retainUnfoldedComments,
  selectVisibleComments,
} from "../selectors.mjs";
import type { DispatchComment, DispatchIssue } from "../types";
import { useDispatchContext, useDispatchWorkspaces } from "../hooks";
import {
  IssueDetail,
  IssueDetailSkeleton,
  IssueDetailUnavailable,
} from "./IssueDetail";

export function DispatchIssueDetailScreen({
  issueId,
  workspaceId,
}: {
  issueId: string;
  workspaceId?: string;
}) {
  const navigate = useNavigate();
  const identity = useIdentityQuery();
  const workspaces = useDispatchWorkspaces();
  const resolvedWorkspace =
    workspaceId ??
    workspaces.workspaces.find((workspace) => workspace.trackerEventCount > 0)
      ?.id ??
    workspaces.workspaces[0]?.id ??
    null;
  const context = useDispatchContext(resolvedWorkspace);
  const canonical = context.issues.find((issue) => issue.issueId === issueId);
  const [optimisticIssue, setOptimisticIssue] =
    React.useState<DispatchIssue | null>(null);
  const [optimisticComments, setOptimisticComments] = React.useState<
    DispatchComment[]
  >([]);
  const [isMutating, setIsMutating] = React.useState(false);
  const issue =
    optimisticIssue?.issueId === issueId ? optimisticIssue : canonical;
  React.useEffect(() => {
    if (
      canonical &&
      optimisticIssue &&
      canonical.tipEvent === optimisticIssue.tipEvent
    )
      setOptimisticIssue(null);
  }, [canonical, optimisticIssue]);
  React.useEffect(() => {
    setOptimisticComments((current) =>
      retainUnfoldedComments(current, context.comments),
    );
  }, [context.comments]);
  const authors = [
    ...new Set([
      ...(issue?.assignee ? [issue.assignee] : []),
      ...context.activity
        .filter((entry) => entry.issueId === issueId)
        .map((entry) => entry.authorPubkey),
      ...context.comments
        .filter((entry) => entry.issueId === issueId)
        .map((entry) => entry.authorPubkey),
    ]),
  ];
  const profiles = useUsersBatchQuery(authors);
  const goBack = () => void navigate({ to: "/dispatch" });

  if (context.isLoading || workspaces.isLoading) return <IssueDetailSkeleton />;
  if (context.error)
    return <IssueDetailUnavailable error={context.error} onBack={goBack} />;
  if (!resolvedWorkspace || !issue)
    return (
      <IssueDetailUnavailable
        error={
          new Error(
            "The issue is not present in the selected workspace's current fold.",
          )
        }
        onBack={goBack}
      />
    );

  const runMutation = async (
    mutation: Parameters<typeof publishDispatchIssueMutation>[0]["mutation"],
  ) => {
    setIsMutating(true);
    try {
      await publishDispatchIssueMutation({
        issue,
        mutation,
        viewerRole: context.viewerRole,
        onOptimistic: setOptimisticIssue,
        onRollback: () => setOptimisticIssue(null),
        refresh: context.refresh,
      });
      setOptimisticIssue(null);
      toast.success("Issue updated");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Issue update failed",
      );
      throw error;
    } finally {
      setIsMutating(false);
    }
  };
  const addComment = async (body: string) => {
    setIsMutating(true);
    try {
      await publishDispatchComment({
        body,
        issue,
        onOptimistic: (comment) =>
          setOptimisticComments((current) => [...current, comment]),
        onRollback: (eventId) =>
          setOptimisticComments((current) =>
            current.filter((comment) => comment.eventId !== eventId),
          ),
        refresh: context.refresh,
      });
      toast.success("Comment added");
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Comment failed");
      throw error;
    } finally {
      setIsMutating(false);
    }
  };

  return (
    <IssueDetail
      activity={context.activity.filter((entry) => entry.issueId === issueId)}
      comments={selectVisibleComments(
        context.comments,
        optimisticComments,
        issueId,
      )}
      currentPubkey={identity.data?.pubkey}
      issue={issue}
      issues={context.issues}
      isMutating={isMutating}
      onAddComment={addComment}
      onBack={goBack}
      onMutate={runMutation}
      onOpenIssue={(target) =>
        void navigate({
          to: "/dispatch/issue/$issueId",
          params: { issueId: target },
          search: { workspace: resolvedWorkspace },
        })
      }
      profiles={profiles.data?.profiles}
      profilesError={
        profiles.error instanceof Error ? profiles.error.message : null
      }
      projects={context.projects}
      role={context.viewerRole}
      sectionErrors={context.sectionErrors}
      taskLinks={context.linkedTasks.filter((task) => task.issueId === issueId)}
    />
  );
}
