import {
  mkdir,
  readdir,
  readFile,
  writeFile,
  unlink,
  rmdir,
} from "node:fs/promises";
import { join } from "node:path";
import { randomUUID } from "node:crypto";

/** Atomic, host-wide admission. Never expire a live owner's reservation. */
export class HostSlots {
  constructor(
    directory,
    limit = 8,
    { reclaimDead = true, resource = "browser" } = {},
  ) {
    this.directory = directory;
    this.limit = limit;
    this.reclaimDead = reclaimDead;
    this.resource = resource;
  }

  async acquire() {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    for (let index = 0; index < this.limit; index++) {
      const slot = join(this.directory, String(index));
      try {
        await mkdir(slot, { mode: 0o700 });
      } catch (error) {
        if (error.code !== "EEXIST") throw error;
        if (this.reclaimDead) await this.reclaim(slot);
        try {
          await mkdir(slot, { mode: 0o700 });
        } catch (retry) {
          if (retry.code === "EEXIST") continue;
          throw retry;
        }
      }
      const receipt = join(slot, `${randomUUID()}.json`);
      try {
        await writeFile(receipt, JSON.stringify({ pid: process.pid }), {
          flag: "wx",
          mode: 0o600,
        });
      } catch (error) {
        await rmdir(slot);
        throw error;
      }
      let released = false;
      return async () => {
        if (released) return;
        released = true;
        await unlink(receipt);
        await rmdir(slot);
      };
    }
    throw new Error(
      `Host ${this.resource} capacity reached (${this.limit} sessions); close a session first`,
    );
  }

  async reclaim(slot) {
    // A unique receipt prevents two reclaimers from deleting a replacement owner's slot.
    // Empty slots are deliberately not stolen: another process may be publishing its receipt.
    try {
      const entries = await readdir(slot);
      if (entries.length !== 1 || !entries[0].endsWith(".json")) return;
      const receipt = join(slot, entries[0]);
      const { pid } = JSON.parse(await readFile(receipt, "utf8"));
      if (!Number.isSafeInteger(pid) || pid <= 0) return;
      try {
        process.kill(pid, 0);
        return;
      } catch (error) {
        if (error.code !== "ESRCH") return;
      }
      await unlink(receipt);
      await rmdir(slot);
    } catch (error) {
      if (!["ENOENT", "ENOTEMPTY"].includes(error.code)) throw error;
    }
  }
}
