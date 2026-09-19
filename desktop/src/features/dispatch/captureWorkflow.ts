/** Shared channel setting consumed by the relay's Dispatch workflow action. */
export function captureWorkflowDefinition(channel: string, enabled: boolean) {
  return {
    name: "dispatch-capture",
    enabled,
    trigger: {
      on: "reaction_added",
      emoji: "📌",
      filter: `trigger_channel_id == ${JSON.stringify(channel)}`,
    },
    steps: [{ id: "capture", action: "dispatch.create_issue" }],
  };
}
