import assert from "node:assert/strict";
import test from "node:test";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

test("SIGKILL of transport owner drains supervisor, MCP server and grandchild", {
  timeout: 6000,
}, async () => {
  const child = spawn(
    process.execPath,
    [fileURLToPath(new URL("./transport-owner.mjs", import.meta.url))],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  let pids;
  try {
    pids = await new Promise((resolve, reject) => {
      let output = "";
      const timer = setTimeout(
        () => reject(new Error("transport startup timeout")),
        3000,
      );
      child.stdout.on("data", (data) => {
        output += data;
        if (output.includes("\n")) {
          clearTimeout(timer);
          resolve(JSON.parse(output.trim()));
        }
      });
      child.once("error", reject);
    });
    child.kill("SIGKILL");
    await new Promise((resolve) => child.once("exit", resolve));
    const alive = (pid) => {
      try {
        process.kill(pid, 0);
        return true;
      } catch (e) {
        if (e.code === "ESRCH") return false;
        throw e;
      }
    };
    const all = [pids.pid, pids.grandchildPid, pids.supervisorPid];
    const deadline = Date.now() + 2000;
    while (all.some(alive) && Date.now() < deadline) await delay(20);
    assert.ok(
      all.every((pid) => !alive(pid)),
      `orphaned owned PIDs: ${all.filter(alive)}`,
    );
  } finally {
    child.kill("SIGKILL");
    if (pids) {
      try {
        process.kill(-pids.supervisorPid, "SIGKILL");
      } catch {}
    }
  }
});
