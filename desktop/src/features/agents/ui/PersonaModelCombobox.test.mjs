import assert from "node:assert/strict";
import { after, afterEach, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
  pretendToBeVisual: true,
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
  getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
  requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
  cancelAnimationFrame: dom.window.cancelAnimationFrame.bind(dom.window),
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
});
for (const key of [
  "Element",
  "HTMLElement",
  "HTMLInputElement",
  "SVGElement",
  "Node",
  "NodeFilter",
  "Event",
  "CustomEvent",
  "MutationObserver",
])
  globalThis[key] = dom.window[key];

const React = await import("react");
const { render, fireEvent, cleanup, act } = await import(
  "@testing-library/react"
);
const { PersonaModelCombobox } = await import("./PersonaModelCombobox.tsx");
afterEach(cleanup);
after(() => dom.window.close());

test("opening focuses search and parent updates preserve option focus", async () => {
  const props = {
    id: "model",
    value: "a",
    options: [
      { value: "a", label: "Model A" },
      { value: "b", label: "Model B" },
    ],
    placeholder: "Choose a model",
    onValueChange() {},
  };
  const view = render(React.createElement(PersonaModelCombobox, props));
  fireEvent.click(view.getByRole("combobox"));
  const search = await view.findByLabelText("Search models");
  assert.equal(document.activeElement, search);
  const option = view.getByRole("button", { name: "Model B", exact: true });
  await act(async () => option.focus());
  view.rerender(
    React.createElement(PersonaModelCombobox, { ...props, value: "b" }),
  );
  assert.equal(
    document.activeElement,
    option,
    "a render must not refocus search",
  );
});
