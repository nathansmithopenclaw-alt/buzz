import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  type FleetAgentStatus,
  fleetAgentStatusPresentation,
} from "./fleetAgentStatus";

export function FleetAgentStatusChip({
  className,
  status,
  testId,
}: {
  className?: string;
  status: FleetAgentStatus;
  testId?: string;
}) {
  const presentation = fleetAgentStatusPresentation(status);

  return (
    <Badge
      aria-label={`Agent status: ${presentation.label}`}
      className={cn(
        "normal-case tracking-normal",
        presentation.stale && "text-muted-foreground",
        className,
      )}
      data-fleet-status-stale={presentation.stale ? "true" : "false"}
      data-testid={testId}
      variant={presentation.variant}
    >
      {presentation.label}
    </Badge>
  );
}
