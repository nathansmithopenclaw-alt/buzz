import { createHash, randomUUID } from "node:crypto";
import {
  mkdir,
  readdir,
  readFile,
  writeFile,
  rename,
  stat,
} from "node:fs/promises";
import { join } from "node:path";
import { HostSlots } from "./slots.mjs";

/** Named opt-in storage, scoped to the host-supplied community/agent owner. */
export class Profiles {
  constructor(registry, owner) {
    this.directory = join(
      registry,
      "profiles",
      createHash("sha256").update(owner).digest("hex"),
    );
  }
  async acquire(name) {
    if (
      typeof name !== "string" ||
      !/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$/.test(name)
    )
      throw new Error(
        "Profile name must be 1–64 letters, numbers, underscores or hyphens",
      );
    const directory = join(this.directory, name);
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const existing = await readdir(this.directory);
    if (!existing.includes(name) && existing.length >= 16)
      throw new Error("Persistent profile limit reached for this owner (16)");
    await mkdir(directory, { recursive: true, mode: 0o700 });
    const release = await new HostSlots(join(directory, "writer"), 1, {
      resource: "profile writer",
    }).acquire();
    const statePath = join(directory, "storage.json");
    let storageState;
    try {
      if ((await stat(statePath)).size > 5_000_000)
        throw new Error("Persistent profile exceeds the 5 MB storage limit");
      storageState = JSON.parse(await readFile(statePath, "utf8"));
    } catch (error) {
      if (error.code !== "ENOENT") {
        await release();
        throw error;
      }
    }
    return {
      name,
      storageState,
      release,
      save: async (context) => {
        const state = await context.storageState({ indexedDB: true });
        const serialized = JSON.stringify(state);
        if (Buffer.byteLength(serialized) > 5_000_000)
          throw new Error(
            "Profile exceeds the 5 MB storage limit; previous saved state was preserved",
          );
        const temporary = join(directory, `${randomUUID()}.tmp`);
        await writeFile(temporary, serialized, { flag: "wx", mode: 0o600 });
        await rename(temporary, statePath);
      },
    };
  }
}
