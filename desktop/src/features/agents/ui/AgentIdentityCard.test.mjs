import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

import { AgentIdentityCard } from "./AgentIdentityCard.tsx";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

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

test("renders the runtime and model before an optional description", async () => {
  const { render } = await import("@testing-library/react");
  const React = await import("react");
  const { container } = render(
    React.createElement(AgentIdentityCard, {
      ariaLabel: "Patch agent profile",
      avatar: React.createElement("div"),
      dataTestId: "patch-card",
      label: "Patch",
      modelLabel: "codex · gpt-5.6",
      subtitle: "Builds and verifies scoped engineering changes.",
    }),
  );

  const lines = [...container.querySelectorAll("span")].map(
    (element) => element.textContent,
  );
  assert.deepEqual(lines, [
    "Patch",
    "codex · gpt-5.6",
    "Builds and verifies scoped engineering changes.",
  ]);
});

test("keeps the runtime and model visible without a description", async () => {
  const { render } = await import("@testing-library/react");
  const React = await import("react");
  const { container } = render(
    React.createElement(AgentIdentityCard, {
      ariaLabel: "Patch agent profile",
      avatar: React.createElement("div"),
      dataTestId: "patch-card",
      label: "Patch",
      modelLabel: "codex · gpt-5.6",
    }),
  );

  assert.match(container.textContent, /Patchcodex · gpt-5\.6/);
});
