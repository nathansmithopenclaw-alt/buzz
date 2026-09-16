// A compacted/pruned transcript cannot authorize reuse of old UI references.
// The host still owns generation, lease, takeover and element validation.
export function guardObservations(ctx, owner) {
  let epoch = 0;
  const observed = new Map();
  const prefix = "mcp__buzz-computer-use__";
  const nativeReads = new Set([
    "native_get_accessibility_tree",
    "native_get_window_state",
    "native_get_desktop_state",
    "native_zoom",
  ]);
  const nativeInputs = new Set(
    [
      "launch_app",
      "bring_to_front",
      "click",
      "double_click",
      "right_click",
      "drag",
      "move_cursor",
      "scroll",
      "hotkey",
      "press_key",
      "type_text",
      "set_value",
      "invoke_menu",
      "set_window_frame",
      "clipboard_write",
    ].map((name) => `native_${name}`),
  );
  ctx.on("session/event", (session, event) => {
    if (session !== owner.session) return;
    if (
      ["compaction/start", "compaction/prune", "compaction/summary"].includes(
        event.type,
      )
    ) {
      epoch++;
      observed.clear();
    }
  });
  ctx.tools.guard((exec) => {
    if (exec.agent !== owner || epoch === 0 || !exec.name.startsWith(prefix))
      return;
    const name = exec.name.slice(prefix.length);
    const key =
      name === "browser_action"
        ? exec.arguments?.session
        : nativeInputs.has(name)
          ? "native"
          : undefined;
    if (key !== undefined && observed.get(key) !== epoch)
      return "Transcript compacted: obtain a fresh observation before acting";
  });
  ctx.on("tools/result", (exec, result) => {
    if (exec.agent !== owner || result.isError || !exec.name.startsWith(prefix))
      return;
    const name = exec.name.slice(prefix.length);
    if (name === "browser_observe")
      observed.set(exec.arguments?.session, epoch);
    if (nativeReads.has(name)) observed.set("native", epoch);
  });
}
