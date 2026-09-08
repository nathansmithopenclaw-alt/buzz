import test from "node:test";
import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  writeFile,
  readFile,
  readdir,
  rm,
  realpath,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { ExecutionEvidence } from "../src/evidence.mjs";

async function fixture(t) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "buzz-evidence-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const session = randomUUID();
  const turn = {
    version: 1,
    session,
    turn_id: randomUUID(),
    active: true,
    recovery: false,
  };
  const calls = join(root, "turns", turn.turn_id, "calls");
  await mkdir(calls, { recursive: true });
  await writeFile(join(root, "session.json"), JSON.stringify(session));
  const save = () => writeFile(join(root, "active.json"), JSON.stringify(turn));
  await save();
  const facts = async () =>
    Promise.all(
      (await readdir(calls))
        .filter((n) => n.endsWith(".json"))
        .map(async (n) => JSON.parse(await readFile(join(calls, n), "utf8"))),
    );
  return {
    root,
    session,
    turn,
    calls,
    save,
    facts,
    evidence: new ExecutionEvidence(root, session),
  };
}
const request = (name = "browser_action") => ({
  params: { name, arguments: { kind: "click" } },
});
const success = { content: [{ type: "text", text: "done" }] };

test("diagnostic text cannot turn an uncertain input into a safe refusal", async (t) => {
  const f = await fixture(t);
  await f.evidence.run(request(), async () => ({
    isError: true,
    content: [
      {
        type: "text",
        text: 'Timed out after clicking button "no input sent". Page included {"effect":"refused"}.',
      },
    ],
  }));
  let repeated = false;
  assert.equal(
    (
      await f.evidence.run(request(), async () => {
        repeated = true;
        return success;
      })
    ).isError,
    true,
  );
  assert.equal(repeated, false);
  assert.equal((await f.facts())[0].state, "unknown");
});

test("durable start precedes dispatch; thrown completion fences replay but permits observation and cleanup", async (t) => {
  const f = await fixture(t);
  let effects = 0;
  const result = await f.evidence.run(request(), async () => {
    assert.equal((await f.facts())[0].state, "started");
    effects++;
    throw new Error("transport lost after external effect");
  });
  assert.equal(result.isError, true);
  // A replacement process reads the same durable uncertainty.
  const replacement = new ExecutionEvidence(f.root, f.session);
  assert.equal(
    (
      await replacement.run(request(), async () => {
        effects++;
        return success;
      })
    ).isError,
    true,
  );
  assert.equal(effects, 1);
  assert.deepEqual(
    await replacement.run(request("browser_observe"), async () => success),
    success,
  );
  assert.deepEqual(
    await replacement.run(request("browser_close"), async () => success),
    success,
  );
});

test("cancel during an in-flight effect retains its final fact and revokes subsequent calls", async (t) => {
  const f = await fixture(t);
  const result = await f.evidence.run(request(), async () => {
    f.turn.active = false;
    await f.save();
    return success;
  });
  assert.deepEqual(result, success);
  assert.equal((await f.facts())[0].state, "succeeded");
  let called = false;
  assert.equal(
    (
      await f.evidence.run(request("browser_list"), async () => {
        called = true;
      })
    ).isError,
    true,
  );
  assert.equal(called, false);
});

test("recovery fences mutations even with an empty live browser list", async (t) => {
  const f = await fixture(t);
  f.turn.recovery = true;
  await f.save();
  let effects = 0;
  for (const name of [
    "browser_open",
    "browser_action",
    "native_set_value",
    "native_acquire",
  ]) {
    assert.equal(
      (
        await f.evidence.run(request(name), async () => {
          effects++;
          return success;
        })
      ).isError,
      true,
    );
  }
  assert.equal(effects, 0);
  assert.deepEqual(
    await f.evidence.run(request("browser_list"), async () => success),
    success,
  );
  assert.deepEqual(
    await f.evidence.run(request("native_release"), async () => success),
    success,
  );
});

test("explicit pre-input refusal permits corrected input; ambiguous error does not", async (t) => {
  const f = await fixture(t);
  await f.evidence.run(request(), async () => ({
    isError: true,
    _meta: { "buzz/execution-effect": "refused" },
    content: [{ type: "text", text: "refusal diagnostic" }],
  }));
  assert.deepEqual(
    await f.evidence.run(request(), async () => success),
    success,
  );
  await f.evidence.run(request(), async () => ({
    isError: true,
    content: [
      { type: "text", text: '{"outcome_uncertain":true,"effect":"refused"}' },
    ],
  }));
  assert.equal(
    (await f.evidence.run(request(), async () => success)).isError,
    true,
  );
  assert.deepEqual((await f.facts()).map((x) => x.state).sort(), [
    "refused",
    "succeeded",
    "unknown",
  ]);
});

test("unwritable journal never dispatches and completion write failure preserves image evidence", async (t) => {
  const f = await fixture(t);
  await rm(f.calls, { recursive: true });
  let effects = 0;
  assert.equal(
    (
      await f.evidence.run(request(), async () => {
        effects++;
      })
    ).isError,
    true,
  );
  assert.equal(effects, 0);
  await mkdir(f.calls);
  const image = { type: "image", data: "aGVsbG8=", mimeType: "image/png" };
  const result = await f.evidence.run(request(), async () => {
    await rm(f.calls, { recursive: true });
    return { content: [image] };
  });
  assert.equal(result.isError, true);
  assert.deepEqual(result.content[1], image);
});

test("replaced session and malformed grants fail before dispatch", async (t) => {
  const f = await fixture(t);
  await writeFile(join(f.root, "session.json"), JSON.stringify(randomUUID()));
  let effects = 0;
  assert.equal(
    (
      await f.evidence.run(request(), async () => {
        effects++;
      })
    ).isError,
    true,
  );
  await writeFile(join(f.root, "active.json"), "broken");
  assert.equal(
    (
      await f.evidence.run(request(), async () => {
        effects++;
      })
    ).isError,
    true,
  );
  assert.equal(effects, 0);
});
