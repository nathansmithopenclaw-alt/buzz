import assert from "node:assert/strict";
import test from "node:test";
import { channelBrowserChannels } from "./channelBrowserChannels.ts";

function channel(name, overrides = {}) {
  return {
    id: name,
    name,
    description: "",
    channelType: "stream",
    visibility: "open",
    isMember: false,
    archivedAt: null,
    memberCount: 1,
    lastMessageAt: null,
    ...overrides,
  };
}

function project(channels, overrides = {}) {
  return channelBrowserChannels({
    channels,
    query: "",
    tab: "all",
    sort: "alpha",
    ...overrides,
  }).map(({ id }) => id);
}

test("a new unmatched query cannot select a previous search result", () => {
  const channels = [channel("agents"), channel("design"), channel("general")];
  assert.deepEqual(project(channels), ["agents", "design", "general"]);
  assert.deepEqual(project(channels, { query: "new-unmatched-channel" }), []);
  assert.deepEqual(project(channels, { query: "design" }), ["design"]);
});

test("all, joined and archived tabs preserve channel visibility", () => {
  const channels = [
    channel("public"),
    channel("joined", { isMember: true }),
    channel("private-joined", { visibility: "private", isMember: true }),
    channel("private-hidden", { visibility: "private" }),
    channel("archived-joined", { archivedAt: 1, isMember: true }),
    channel("archived-hidden", { archivedAt: 1 }),
    channel("dm", { channelType: "dm", isMember: true }),
  ];
  assert.deepEqual(project(channels), [
    "archived-joined",
    "joined",
    "private-joined",
    "public",
  ]);
  assert.deepEqual(project(channels, { tab: "joined" }), [
    "joined",
    "private-joined",
  ]);
  assert.deepEqual(project(channels, { tab: "archived" }), ["archived-joined"]);
});

test("forum and stream modes use the same filter for display and actions", () => {
  const channels = [
    channel("stream"),
    channel("forum", { channelType: "forum" }),
  ];
  assert.deepEqual(project(channels, { channelTypeFilter: "stream" }), [
    "stream",
  ]);
  assert.deepEqual(project(channels, { channelTypeFilter: "forum" }), [
    "forum",
  ]);
});

test("search relevance precedes member sorting with stable alphabetical ties", () => {
  const channels = [
    channel("design", { memberCount: 1 }),
    channel("design-z", { memberCount: 20 }),
    channel("design-a", { memberCount: 20 }),
  ];
  assert.deepEqual(project(channels, { sort: "members" }), [
    "design-a",
    "design-z",
    "design",
  ]);
  assert.deepEqual(project(channels, { query: "design", sort: "members" }), [
    "design",
    "design-a",
    "design-z",
  ]);
});
