import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { shouldObserveManagedAgents } from "./observerRelayStore.ts";

test("observer ingestion opens for a cold stopped managed agent", () => {
  assert.equal(
    shouldObserveManagedAgents([{ pubkey: "aa", status: "stopped" }]),
    true,
  );
});

test("observer ingestion stays closed when there are no owned agents", () => {
  assert.equal(shouldObserveManagedAgents([]), false);
});

test("the app-level management listener opens the ephemeral observer subscription", async () => {
  const source = await readFile(
    new URL("./observerRelayStore.ts", import.meta.url),
    "utf8",
  );
  const managementSubscription = source.slice(
    source.indexOf("export function subscribeAgentManagementRequests"),
    source.indexOf("export function subscribeControlResults"),
  );

  assert.match(managementSubscription, /ensureRelayObserverSubscription\(\)/);
});
