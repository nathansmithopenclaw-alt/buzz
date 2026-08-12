export function DispatchIssueDetailStub({ issueId }: { issueId: string }) {
  return (
    <main className="h-full p-6" data-testid="dispatch-issue-detail-mount">
      <h1 className="text-lg font-semibold">Issue detail</h1>
      <p className="mt-2 font-mono text-sm text-muted-foreground">{issueId}</p>
    </main>
  );
}
