import assert from "node:assert/strict";
import test from "node:test";
import { modelEffortPresentation } from "./modelEffortPresentation.ts";
test("native capability separates model label from effort without rewriting saved values", () => {
  const options = [
    { id: "astra", label: "Astra", badge: "Available now" },
    { id: "astra[high]", label: "astra[high]" },
    { id: "sol", label: "Sol" },
  ];
  assert.deepEqual(modelEffortPresentation(options, "astra[high]", true), [
    { id: "astra[high]", label: "Astra", badge: "Available now" },
    { id: "sol", label: "Sol" },
  ]);
  assert.equal(modelEffortPresentation(options, "astra[high]", false), options);
  assert.equal(modelEffortPresentation(options, "astra", true), options);
});
