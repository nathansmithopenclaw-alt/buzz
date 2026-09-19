import assert from "node:assert/strict";
import test from "node:test";
import { dispatchPopulationProbeFilter } from "./e2eDispatchPopulation.ts";

test("population fixtures preserve every workspace in a batched probe", () => {
  const filters = ["one", "two"].map((id) => ({
    kinds: [40020, 40021],
    "#h": [id],
    limit: 1,
  }));
  assert.deepEqual(dispatchPopulationProbeFilter(filters), {
    kinds: [40020, 40021],
    "#h": ["one", "two"],
    limit: 2,
  });
  assert.deepEqual(filters[0]["#h"], ["one"]);
  assert.equal(dispatchPopulationProbeFilter(filters.slice(0, 1)), null);
  assert.equal(
    dispatchPopulationProbeFilter([...filters, { ...filters[0], limit: 500 }]),
    null,
  );
});
