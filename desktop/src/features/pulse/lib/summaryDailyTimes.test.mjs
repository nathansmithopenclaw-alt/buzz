import assert from "node:assert/strict";
import test from "node:test";
import { dailyTimesToCrons } from "./summaryDailyTimes.ts";
import { summaryScheduleError } from "./summarySchedule.ts";
test("daily times preserve exact minute/hour pairs without extra runs", () => {
  assert.deepEqual(dailyTimesToCrons(["09:15", "17:45", "13:15"]), [
    "15 9,13 * * *",
    "45 17 * * *",
  ]);
  assert.equal(
    summaryScheduleError({ on: "schedule", dailyTimes: ["09:15", "17:45"] }),
    null,
  );
});
test("daily times reject duplicate, empty and malformed times", () => {
  for (const times of [[], ["09:00", "09:00"], ["24:00"], ["9:00"], ["09:60"]])
    assert.throws(() => dailyTimesToCrons(times));
});
