import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
test("every runtime package file is embedded for desktop provisioning", async () => {
  const rust = await readFile(
    new URL("../../dsh_profile.rs", import.meta.url),
    "utf8",
  );
  for (const name of await readdir(
    new URL("../packages/dsh-acp-buzz/", import.meta.url),
  )) {
    assert.ok(
      rust.includes(`dsh_profile_resources/packages/dsh-acp-buzz/${name}`),
      `missing embedded runtime file: ${name}`,
    );
  }
});
