import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() =>
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
  }),
);
afterEach(async () => {
  (await import("@testing-library/react")).cleanup();
  document.body.replaceChildren();
});
after(() => dom.window.close());

async function prepare(markup = "", selector = null) {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { useComposerAutofocus } = await import("./useComposerAutofocus.ts");
  document.body.innerHTML = markup;
  const target = selector ? document.querySelector(selector) : null;
  target?.focus();
  const calls = [];
  const focus = () => calls.push("focused");
  function Harness({ callback, draftKey = "channel-a", disabled = false }) {
    useComposerAutofocus(callback, draftKey, disabled);
    return null;
  }
  const view = render(React.createElement(Harness, { callback: () => {} }));
  return {
    calls,
    target,
    ready: (props = {}) =>
      view.rerender(
        React.createElement(Harness, { callback: focus, ...props }),
      ),
  };
}

test("editor becoming ready preserves focus in an open popover switch", async () => {
  const h = await prepare(
    '<div role="dialog" data-state="open"><button role="switch">Voice</button></div>',
    "button",
  );
  h.ready();
  assert.deepEqual(h.calls, []);
  assert.equal(document.activeElement, h.target);
});

for (const role of ["menu", "listbox"]) {
  test(`navigation does not steal focus from an open ${role}`, async () => {
    const h = await prepare(
      `<div role="${role}"><button>Choice</button></div>`,
      "button",
    );
    h.ready({ draftKey: "channel-b" });
    assert.deepEqual(h.calls, []);
  });
}

test("an expanded popup trigger keeps focus but ordinary navigation can autofocus", async () => {
  const h = await prepare(
    '<button aria-haspopup="dialog" aria-expanded="true">Open</button>',
    "button",
  );
  h.ready();
  assert.deepEqual(h.calls, []);
  h.target.setAttribute("aria-expanded", "false");
  h.ready({ draftKey: "channel-b" });
  assert.deepEqual(h.calls, ["focused"]);
});

test("a closing overlay does not prevent navigation autofocus", async () => {
  const h = await prepare(
    '<div role="dialog" data-state="closed"><button>Old action</button></div>',
    "button",
  );
  h.ready();
  assert.deepEqual(h.calls, ["focused"]);
});

test("existing text-entry focus remains protected", async () => {
  const h = await prepare('<input value="draft">', "input");
  h.ready();
  assert.deepEqual(h.calls, []);
});

test("ready navigation autofocuses while transient disabled changes do not retrigger", async () => {
  const h = await prepare();
  h.ready({ disabled: true });
  assert.deepEqual(h.calls, []);
  h.ready({ disabled: false });
  assert.deepEqual(h.calls, []);
  h.ready({ draftKey: "channel-b" });
  assert.deepEqual(h.calls, ["focused"]);
});
