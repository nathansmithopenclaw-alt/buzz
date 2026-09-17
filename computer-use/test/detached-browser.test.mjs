import test from "node:test";
import assert from "node:assert/strict";
import { assertDetachedBrowser } from "../../scripts/require-detached-browser.mjs";

function inspectChain(entries) {
  return (pid) => {
    assert.ok(entries[pid], `Unexpected process read: ${pid}`);
    return entries[pid];
  };
}

for (const app of ["ChatGPT", "Codex"]) {
  test(`refuses an indirect ${app} Desktop browser descendant`, () => {
    assert.throws(
      () =>
        assertDetachedBrowser({
          platform: "darwin",
          pid: 30,
          inspect: inspectChain({
            30: { parentPid: 20, executable: "/tools/node" },
            20: { parentPid: 10, executable: "/bin/bash" },
            10: {
              parentPid: 1,
              executable: `/Applications/${app}.app/Contents/MacOS/${app}`,
            },
          }),
        }),
      /extra Dock icons/,
    );
  });
}

test("permits a finite launchd-owned runner", () => {
  assert.doesNotThrow(() =>
    assertDetachedBrowser({
      platform: "darwin",
      pid: 30,
      inspect: inspectChain({
        30: { parentPid: 20, executable: "/tools/node" },
        20: { parentPid: 1, executable: "/usr/bin/python3" },
      }),
    }),
  );
});

test("does not inspect process ancestry on other platforms", () => {
  for (const platform of ["linux", "win32"]) {
    assertDetachedBrowser({
      platform,
      inspect: () => assert.fail("Unexpected ps call"),
    });
  }
});

test("unreadable ancestry fails closed", () => {
  assert.throws(
    () =>
      assertDetachedBrowser({
        platform: "darwin",
        pid: 30,
        inspect: () => {
          throw new Error("ps failed");
        },
      }),
    /refusing browser launch/,
  );
});

test("invalid or cyclic ancestry fails closed", () => {
  for (const parentPid of [30, -1, NaN]) {
    assert.throws(
      () =>
        assertDetachedBrowser({
          platform: "darwin",
          pid: 30,
          inspect: () => ({ parentPid, executable: "/tools/node" }),
        }),
      /refusing browser launch/,
    );
  }
});
