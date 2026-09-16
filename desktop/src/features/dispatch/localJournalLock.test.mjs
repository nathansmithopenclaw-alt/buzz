import test from "node:test";
import assert from "node:assert/strict";
import { withLocalJournalLock } from "./localJournalLock.ts";
test("separate journals can be nested while writes to one journal remain ordered", {
  timeout: 1000,
}, async () => {
  const order = [];
  await Promise.all([
    withLocalJournalLock("assignment", async () => {
      order.push("assignment-start");
      await withLocalJournalLock("issue", async () => order.push("issue-link"));
      order.push("assignment-end");
    }),
    withLocalJournalLock("assignment", async () =>
      order.push("next-assignment"),
    ),
  ]);
  assert.deepEqual(order, [
    "assignment-start",
    "issue-link",
    "assignment-end",
    "next-assignment",
  ]);
});
