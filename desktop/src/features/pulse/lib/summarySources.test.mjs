import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultSourceConfig,
  formatLocalTime,
  isConversationChecked,
  listsFromChecked,
  parseLocalTime,
  partitionLiveConversations,
  toggleConversation,
} from "./summarySources.ts";

const liveChannel = {
  id: "c1",
  name: "buzz-dev",
  kind: "channel",
  archived: false,
};
const liveDm = {
  id: "d1",
  name: "Nathan",
  kind: "dm",
  archived: false,
};
const archived = {
  id: "c2",
  name: "old",
  kind: "channel",
  archived: true,
};

test("default source config includes every live conversation", () => {
  const config = defaultSourceConfig([liveChannel, liveDm, archived]);
  assert.equal(config.autoInclude, true);
  assert.deepEqual(config.include, ["c1", "d1"]);
  assert.deepEqual(config.exclude, []);
});

test("auto-include treats unseen live conversations as checked", () => {
  const config = {
    autoInclude: true,
    include: ["c1"],
    exclude: [],
  };
  assert.equal(isConversationChecked(config, liveDm), true);
  assert.equal(isConversationChecked(config, archived), false);
});

test("unchecking a conversation records an exclude", () => {
  const next = toggleConversation(
    { autoInclude: true, include: ["c1", "d1"], exclude: [] },
    liveDm,
    false,
  );
  assert.equal(isConversationChecked(next, liveDm), false);
  assert.deepEqual(next.exclude, ["d1"]);
});

test("archived conversations never appear in live partitions", () => {
  const { channels, dms } = partitionLiveConversations([
    liveChannel,
    liveDm,
    archived,
  ]);
  assert.deepEqual(
    channels.map((c) => c.id),
    ["c1"],
  );
  assert.deepEqual(
    dms.map((c) => c.id),
    ["d1"],
  );
});

test("listsFromChecked drops archived ids even if they were included", () => {
  const lists = listsFromChecked([liveChannel, liveDm, archived], {
    autoInclude: true,
    include: ["c1", "c2", "d1"],
    exclude: [],
  });
  assert.deepEqual(lists.channels, ["c1"]);
  assert.deepEqual(lists.dms, ["d1"]);
});

test("local time formatting round-trips", () => {
  assert.equal(formatLocalTime(9, 0), "09:00");
  assert.deepEqual(parseLocalTime("09:00"), { hour: 9, minute: 0 });
  assert.equal(parseLocalTime("24:00"), null);
});
