import * as React from "react";
import { toast } from "sonner";
import { useRemindLater } from "@/features/reminders/ui/RemindMeLaterProvider";
import { getEventById } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import type { DispatchIssue } from "../types";

/** Reuse the personal reminder flow with the issue's immutable root as target. */
export function IssueReminderButton({ issue }: { issue: DispatchIssue }) {
  const { openReminder } = useRemindLater();
  const [busy, setBusy] = React.useState(false);
  const generation = React.useRef(0);
  React.useEffect(() => {
    generation.current += 1;
    return () => {
      generation.current += 1;
    };
  }, []);
  const open = async () => {
    const current = generation.current;
    setBusy(true);
    try {
      const root = await getEventById(issue.rootEvent);
      if (current !== generation.current) return;
      openReminder({
        eventId: issue.rootEvent,
        channelId: issue.streamId,
        preview: issue.title,
        authorPubkey: root.pubkey,
      });
    } catch {
      if (current === generation.current)
        toast.error("Could not load the issue for a reminder. Try again.");
    } finally {
      if (current === generation.current) setBusy(false);
    }
  };
  return (
    <Button
      size="sm"
      variant="outline"
      disabled={busy}
      onClick={() => void open()}
    >
      Remind me
    </Button>
  );
}
