import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const agent = "a".repeat(64);
const scope = `${"1".repeat(64)}:channel-a:channel`;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("automatic promotion is gated by the preference", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const audienceStore = await import("../lib/persistentAgentAudience.ts");
  const { useAutoPinMentionedAgents } = await import(
    "./useAutoPinMentionedAgents.ts"
  );
  audienceStore.resetPersistentAgentAudienceStore();
  const { result, rerender } = renderHook(
    ({ enabled }) =>
      useAutoPinMentionedAgents({
        audienceScope: scope,
        enabled,
        getDisplayName: () => "Agent Ada",
        onPulse: () => {},
        onTurnOff: () => {},
      }),
    { initialProps: { enabled: false } },
  );

  act(() => result.current.promoteMentionedAgents({ pubkeys: [agent] }));
  assert.deepEqual(
    audienceStore.getPersistentAgentAudienceSnapshot().audiences,
    {},
  );

  rerender({ enabled: true });
  act(() => result.current.promoteMentionedAgents({ pubkeys: [agent] }));
  assert.deepEqual(
    audienceStore.getPersistentAgentAudienceSnapshot().audiences[scope],
    [agent],
  );
});
