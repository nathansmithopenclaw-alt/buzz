import { execFileSync } from "node:child_process";

const desktopExecutable =
  /\/(?:ChatGPT\.app\/Contents\/MacOS\/ChatGPT|Codex\.app\/Contents\/MacOS\/Codex|Buzz\.app\/Contents\/MacOS\/buzz-desktop)$/;

function readProcess(pid) {
  const output = execFileSync(
    "/bin/ps",
    ["-p", String(pid), "-o", "ppid=,comm="],
    {
      encoding: "utf8",
    },
  ).trim();
  const match = /^(\d+)\s+(.+)$/.exec(output);
  if (!match)
    throw new Error(`Cannot read parent of browser runner PID ${pid}`);
  return { parentPid: Number(match[1]), executable: match[2] };
}

/** Prevent macOS browser helpers from registering as extra Desktop Dock apps. */
export function assertDetachedBrowser({
  platform = process.platform,
  pid = process.pid,
  inspect = readProcess,
} = {}) {
  if (platform !== "darwin") return;
  const visited = new Set();
  while (pid > 1) {
    if (!Number.isInteger(pid) || visited.has(pid)) {
      throw new Error(
        "Cannot establish browser runner ancestry; refusing browser launch.",
      );
    }
    visited.add(pid);
    let ancestor;
    try {
      ancestor = inspect(pid);
    } catch (cause) {
      throw new Error(
        "Cannot establish browser runner ancestry; refusing browser launch.",
        { cause },
      );
    }
    if (desktopExecutable.test(ancestor.executable)) {
      throw new Error(
        "Browser tests cannot launch as ChatGPT/Codex/Buzz Desktop descendants on macOS: " +
          "their helpers inherit the app identity and create extra Dock icons. " +
          "Run this check from Terminal or a finite detached launchd job. " +
          "This refusal is not a passing measurement.",
      );
    }
    if (!Number.isInteger(ancestor.parentPid) || ancestor.parentPid < 0) {
      throw new Error(
        "Cannot establish browser runner ancestry; refusing browser launch.",
      );
    }
    pid = ancestor.parentPid;
  }
}

// Playwright globalSetup invokes this before starting any test browser.
export default function requireDetachedBrowser() {
  assertDetachedBrowser();
}
