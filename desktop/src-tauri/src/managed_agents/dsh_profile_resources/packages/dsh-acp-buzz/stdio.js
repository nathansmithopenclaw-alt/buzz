import { fileURLToPath } from "node:url";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
const execFileAsync = promisify(execFile);
import {
  ReadBuffer,
  serializeMessage,
} from "@modelcontextprotocol/sdk/shared/stdio.js";
import { setTimeout as delay } from "node:timers/promises";

// Each trusted stdio server owns a fresh POSIX process group. Closing that
// capability drains ordinary descendants, including a child ignoring SIGTERM.
// Deliberate daemonization is outside the trusted local-server contract.
export class OwnedStdioTransport {
  constructor(config) {
    this.config = config;
    this.buffer = new ReadBuffer({ maxBufferSize: 10 * 1024 * 1024 });
  }
  async start() {
    if (process.platform === "win32")
      throw new Error("owned MCP process groups require POSIX");
    if (this.child) throw new Error("transport already started");
    this.child = spawn(
      process.execPath,
      [
        fileURLToPath(new URL("./supervisor.js", import.meta.url)),
        this.config.command,
        ...this.config.args,
      ],
      {
        cwd: this.config.cwd,
        env: this.config.env,
        shell: false,
        detached: true,
        stdio: ["pipe", "pipe", "ignore", "pipe"],
      },
    );
    const child = this.child;
    child.once("close", () => {
      this.exited = true;
      this.close().catch((error) => this.onerror?.(error));
    });
    child.stdout.on("data", (chunk) => {
      try {
        this.buffer.append(chunk);
        let message = this.buffer.readMessage();
        while (message !== null) {
          this.onmessage?.(message);
          message = this.buffer.readMessage();
        }
      } catch (error) {
        this.onerror?.(error);
        this.close().catch((error) => this.onerror?.(error));
      }
    });
    child.stdin.on("error", (error) => this.onerror?.(error));
    child.stdout.on("error", (error) => this.onerror?.(error));
    await new Promise((resolve, reject) => {
      child.once("spawn", resolve);
      child.once("error", reject);
    });
    this.pid = child.pid;
  }
  async send(message) {
    if (this.closing || !this.child?.stdin.writable)
      throw new Error("MCP transport is closed");
    await new Promise((resolve, reject) =>
      this.child.stdin.write(serializeMessage(message), (error) =>
        error ? reject(error) : resolve(),
      ),
    );
  }
  alive() {
    if (!this.pid) return false;
    try {
      process.kill(-this.pid, 0);
      return true;
    } catch (error) {
      if (error.code === "ESRCH") return false;
      throw error;
    }
  }
  signal(signal) {
    if (!this.pid) return;
    try {
      process.kill(-this.pid, signal);
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  }
  async groupPids() {
    if (!this.pid) return [];
    const { stdout } = await execFileAsync("/bin/ps", ["-axo", "pid=,pgid="], {
      timeout: 1000,
      maxBuffer: 1024 * 1024,
    });
    return stdout
      .trim()
      .split("\n")
      .map((line) => line.trim().split(/\s+/).map(Number))
      .filter(([, group]) => group === this.pid)
      .map(([pid]) => pid);
  }
  pidAlive(pid) {
    try {
      process.kill(pid, 0);
      return true;
    } catch (error) {
      if (error.code === "ESRCH") return false;
      throw error;
    }
  }
  close() {
    if (this.closing) return this.closing;
    this.closing = (async () => {
      let pids = [],
        snapshotError;
      try {
        pids = await this.groupPids();
      } catch (error) {
        snapshotError = error;
      }
      this.child?.stdin.end();
      this.signal("SIGTERM");
      const grace = Date.now() + 250;
      while (this.alive() && Date.now() < grace) await delay(10);
      if (this.alive()) this.signal("SIGKILL");
      const deadline = Date.now() + 2000;
      while (
        (this.alive() || pids.some((pid) => this.pidAlive(pid))) &&
        Date.now() < deadline
      )
        await delay(10);
      if (this.alive() || pids.some((pid) => this.pidAlive(pid)))
        throw new Error("MCP process group or descendant PID failed to exit");
      if (snapshotError)
        throw new Error("MCP descendant snapshot failed", {
          cause: snapshotError,
        });
      this.buffer.clear();
      this.onclose?.();
    })();
    return this.closing;
  }
}
