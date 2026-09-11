import assert from "node:assert/strict";
import test from "node:test";
import { DispatchHistoryGapError } from "./history.ts";
import { loadDispatchContext } from "./contextLoader.ts";
const workspace = "2708da4e-e077-4b7a-9f1e-893cbe87b692";
const viewer = "a".repeat(64);
const context = {
  issues: [],
  activity: [],
  comments: [],
  projects: [],
  linkedTasks: [],
  diagnostics: [],
  taskDiagnostics: [],
  viewerRole: "member",
  snapshotAt: new Date(100000).toISOString(),
};
function fixture() {
  const calls = [];
  const membership = { id: "members", kind: 39002, tags: [["d", workspace]] };
  const history = {
    fetch: async () => {
      calls.push("membership");
      return [membership];
    },
    fetchMulti: async () => [],
    fetchExhaustively: async (filter) => {
      calls.push(filter);
      return [];
    },
  };
  const native = {
    membership: async () => {
      calls.push("membership");
      return membership;
    },
    cursor: async () => 100,
    preview: async (input) => {
      assert.equal(input.membershipEvent, membership);
      calls.push("preview");
      return { ...context, viewerRole: "none" };
    },
    resume: async (input) => {
      calls.push({ resume: input });
      return context;
    },
    fold: async (input) => {
      calls.push({ fold: input });
      return context;
    },
    incremental: async () => context,
  };
  return { calls, history, native };
}
test("verified local preview arrives before slow relay history and stays read-only", async () => {
  const f = fixture();
  let release;
  const blocked = new Promise((resolve) => {
    release = resolve;
  });
  f.history.fetchExhaustively = async (filter) => {
    f.calls.push(filter);
    await blocked;
    return [];
  };
  let preview;
  let announced;
  const shown = new Promise((resolve) => {
    announced = resolve;
  });
  const result = loadDispatchContext(
    workspace,
    viewer,
    () => {},
    (value) => {
      preview = value;
      announced();
    },
    f,
  );
  await Promise.race([
    shown,
    new Promise((_, reject) => {
      const timer = setTimeout(
        () =>
          reject(
            Error(
              "Cached preview never appeared before relay history completed",
            ),
          ),
        1000,
      );
      shown.then(() => clearTimeout(timer));
    }),
  ]);
  assert.equal(preview.viewerRole, "none");
  assert.deepEqual(f.calls.slice(0, 2), ["membership", "preview"]);
  assert.equal(
    f.calls.some((call) => call.resume),
    false,
  );
  release();
  assert.equal((await result).viewerRole, "member");
});
test("missing or revoked membership cannot fall back to cached private issues", async () => {
  const f = fixture();
  f.history.fetch = async () => [];
  f.native.membership = async () => null;
  await assert.rejects(
    loadDispatchContext(
      workspace,
      viewer,
      () => {},
      () => assert.fail("private preview"),
      f,
    ),
    /membership snapshot is unavailable/,
  );
  assert.equal(f.calls.includes("preview"), false);
});

test("saved preview does not wait for the startup WebSocket history queue", async () => {
  const f = fixture();
  f.history.fetch = async () => {
    throw new Error("WebSocket startup history is still restoring");
  };
  let preview;
  await loadDispatchContext(
    workspace,
    viewer,
    () => {},
    (value) => {
      preview = value;
    },
    f,
  );
  assert.equal(preview.viewerRole, "none");
});
test("failed records history must not advance the shared warm cursor", async () => {
  const f = fixture();
  f.history.fetchExhaustively = async (filter) => {
    if (filter.kinds.includes(40027)) throw Error("records offline");
    return [];
  };
  const result = await loadDispatchContext(
    workspace,
    viewer,
    () => {},
    () => {},
    f,
  );
  assert.equal(f.calls.find((call) => call.resume).resume.advanceCursor, false);
  assert.equal(result.sectionErrors.records, "records offline");
});
test("cancelling a scope during cache reconstruction publishes no preview or fold", async () => {
  const f = fixture();
  const controller = new AbortController();
  f.native.preview = async () => {
    controller.abort();
    return context;
  };
  await assert.rejects(
    loadDispatchContext(
      workspace,
      viewer,
      () => {},
      () => assert.fail("stale preview"),
      { ...f, signal: controller.signal },
    ),
    /abort/i,
  );
  assert.equal(
    f.calls.some((call) => call.resume || call.fold),
    false,
  );
});
test("an absent or unusable cache still loads fresh history normally", async () => {
  const f = fixture();
  f.native.cursor = async () => null;
  f.native.preview = async () => null;
  const result = await loadDispatchContext(
    workspace,
    viewer,
    () => {},
    () => assert.fail("no cache"),
    f,
  );
  assert.equal(f.calls.find((call) => call.fold).fold.advanceCursor, true);
  assert.equal(result.viewerRole, "member");
});

test("warm records read only the inclusive cursor delta", async () => {
  const f = fixture();
  await loadDispatchContext(
    workspace,
    viewer,
    () => {},
    () => {},
    f,
  );
  const records = f.calls.find((call) => call.kinds?.includes(40027));
  assert.equal(records.since, 100);
});

test("a records timestamp gap retries every stream from a complete cold history", async () => {
  const f = fixture();
  f.history.fetchExhaustively = async (filter) => {
    f.calls.push(filter);
    if (filter.kinds.includes(40027) && filter.since !== undefined)
      throw new DispatchHistoryGapError();
    return [];
  };
  await loadDispatchContext(
    workspace,
    viewer,
    () => {},
    () => {},
    f,
  );
  assert.equal(
    f.calls.some((call) => call.resume),
    false,
  );
  assert.equal(f.calls.find((call) => call.fold).fold.advanceCursor, true);
  assert.equal(
    f.calls.filter((call) => call.kinds && call.since === undefined).length,
    5,
  );
});
