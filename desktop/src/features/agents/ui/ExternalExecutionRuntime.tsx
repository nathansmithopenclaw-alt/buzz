import { Button } from "@/shared/ui/button";

export function ExternalExecutionRuntime({ onOpen }: { onOpen: () => void }) {
  return (
    <section
      className="space-y-3 rounded-xl border p-4"
      data-testid="external-execution-runtime"
    >
      <h3 className="text-sm font-semibold">Runs in Codex Desktop</h3>
      <p className="text-sm text-muted-foreground">
        Mention this agent in Buzz to send work to its connected task. Model
        settings and active work are managed in Codex Desktop.
      </p>
      <Button variant="outline" onClick={onOpen}>
        Open Codex Desktop
      </Button>
    </section>
  );
}
