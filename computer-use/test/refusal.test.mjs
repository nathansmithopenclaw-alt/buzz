import test from "node:test";
import assert from "node:assert/strict";
import { nativeRefused } from "../src/refusal.mjs";

test("native refusal evidence requires a typed envelope or the exact unsupported AX result", () => {
  const result = (text) => ({
    isError: true,
    content: [{ type: "text", text }],
  });
  assert.equal(
    nativeRefused(
      result('{"code":"same_pid_keyboard_ambiguity","effect":"refused"}'),
    ),
    true,
  );
  assert.equal(
    nativeRefused(
      result(
        "AX action failed: AXUIElementPerformAction(AXPress) returned -25206",
      ),
    ),
    true,
  );
  assert.equal(
    nativeRefused(
      result(
        "Timed out while inspecting text AXUIElementPerformAction(AXPress) returned -25206",
      ),
    ),
    false,
  );
  assert.equal(
    nativeRefused(
      result(
        '{"message":"no input sent","page":{"code":"example","effect":"refused"}}',
      ),
    ),
    false,
  );
  assert.equal(
    nativeRefused({
      ...result('{"code":"ambiguous","effect":"refused"}'),
      structuredContent: { outcome_uncertain: true },
    }),
    false,
  );
});
