import { open, rename, unlink, readdir, lstat } from "node:fs/promises";
import { constants } from "node:fs";
import { isAbsolute, dirname, join } from "node:path";
import { randomUUID } from "node:crypto";

const id = (value) =>
  typeof value === "string" &&
  /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(value);
const readOnly = new Set([
  "computer_status",
  "browser_list",
  "browser_observe",
  "browser_close",
  "native_release",
  "native_check_permissions",
  "native_health_report",
  "native_list_apps",
  "native_list_windows",
  "native_get_accessibility_tree",
  "native_get_window_state",
  "native_get_desktop_state",
  "native_get_screen_size",
  "native_get_cursor_position",
  "native_zoom",
  "native_verify_state",
  "native_clipboard_read",
]);

async function read(path) {
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size > 131072)
      throw new Error("Invalid execution record size");
    const bytes = await file.readFile();
    if (bytes.length > 131072)
      throw new Error("Execution record grew beyond limit");
    return JSON.parse(bytes.toString("utf8"));
  } finally {
    await file.close();
  }
}

async function write(path, value) {
  const bytes = JSON.stringify(value);
  if (Buffer.byteLength(bytes) > 131072)
    throw new Error("Execution record exceeds limit");
  const temp = `${path}.${randomUUID()}.tmp`;
  try {
    const file = await open(temp, "wx", 0o600);
    try {
      await file.writeFile(bytes);
      await file.sync();
    } finally {
      await file.close();
    }
    await rename(temp, path);
    const directory = await open(dirname(path), "r");
    try {
      await directory.sync();
    } finally {
      await directory.close();
    }
  } finally {
    await unlink(temp).catch(() => {});
  }
}

/** Persist before dispatch; missing results remain unknown across process death. */
export class ExecutionEvidence {
  constructor(root, session) {
    if (!isAbsolute(root) || !id(session))
      throw new Error("Invalid execution grant");
    this.root = root;
    this.session = session;
    this.queue = Promise.resolve();
  }

  run(request, execute) {
    const result = this.queue.then(() => this.record(request, execute));
    this.queue = result.catch(() => {});
    return result;
  }

  async record(request, execute) {
    let fact;
    let path;
    let result;
    let dispatched = false;
    try {
      for (let p = this.root; ; p = dirname(p)) {
        if ((await lstat(p)).isSymbolicLink())
          throw new Error("Unsafe execution directory");
        if (p === dirname(p)) break;
      }
      const current = await read(join(this.root, "session.json"));
      const turn = await read(join(this.root, "active.json"));
      if (
        current !== this.session ||
        turn.session !== this.session ||
        !turn.active ||
        !id(turn.turn_id)
      )
        throw new Error(
          "Execution grant is inactive or replaced; no tool dispatched",
        );
      const tool = request.params.name;
      if (
        !/^(browser_|native_|computer_status)/.test(tool) ||
        tool.length > 100
      )
        throw new Error("Unknown execution tool");
      const mutation = !readOnly.has(tool);
      if (turn.recovery && mutation)
        throw new Error(
          "Cancelled-turn recovery fences new mutations to prevent duplicate effects. Inspect host execution evidence, close/release resources and report. A new explicit user task is required to resume input; no tool dispatched.",
        );
      const directory = join(this.root, "turns", turn.turn_id, "calls");
      for (let p = directory; ; p = dirname(p)) {
        if ((await lstat(p)).isSymbolicLink())
          throw new Error("Unsafe execution calls directory");
        if (p === dirname(p)) break;
      }
      const records = await readdir(directory);
      if (records.length >= 512)
        throw new Error(
          "Execution journal call limit reached; no tool dispatched",
        );
      if (mutation) {
        for (const name of records.filter((name) => name.endsWith(".json"))) {
          const prior = await read(join(directory, name));
          if (prior.mutation && ["started", "unknown"].includes(prior.state))
            throw new Error(
              "A prior input has an unknown outcome; new mutations are fenced, no tool dispatched. Inspect and report instead of replaying input.",
            );
        }
      }
      path = join(directory, `${randomUUID()}.json`);
      fact = {
        version: 1,
        tool,
        mutation,
        state: "started",
        started_at: new Date().toISOString(),
      };
      if (
        tool === "browser_action" &&
        typeof request.params.arguments?.kind === "string" &&
        request.params.arguments.kind.length <= 32
      )
        fact.action = request.params.arguments.kind;
      await write(path, fact);
      const latest = await read(join(this.root, "active.json"));
      const grant = await read(join(this.root, "session.json"));
      if (
        !latest.active ||
        latest.turn_id !== turn.turn_id ||
        latest.session !== this.session ||
        grant !== this.session
      ) {
        fact.state = "refused";
        await write(path, fact);
        throw new Error(
          "Execution grant revoked before dispatch; no tool dispatched",
        );
      }
      dispatched = true;
      result = await execute();
      const text = (result.content ?? [])
        .filter((b) => b.type === "text")
        .map((b) => b.text)
        .join("\n");
      // Refusal evidence must explicitly establish that no input was sent.
      fact.state = result.isError
        ? !/"outcome_uncertain"\s*:\s*true/.test(text) &&
          /no input sent|no tool dispatched|Invalid tool arguments|Observe the target after acquiring ownership|"effect":"refused"|"status":"refused"|AXUIElementPerformAction\(AXPress\) returned -25206/.test(
            text,
          )
          ? "refused"
          : "unknown"
        : "succeeded";
      fact.finished_at = new Date().toISOString();
      await write(path, fact);
      return result;
    } catch (error) {
      // A durable 'started' record intentionally survives a failed completion write.
      return {
        ...result,
        isError: true,
        content: [
          {
            type: "text",
            text: `Execution evidence: ${error.message}${dispatched ? "; dispatched outcome may be unknown. Do not repeat input." : "; no tool dispatched."}`,
          },
          ...(result?.content ?? []),
        ],
      };
    }
  }
}
