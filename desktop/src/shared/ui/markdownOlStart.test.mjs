import assert from "node:assert/strict";
import test from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { createMarkdownComponents } from "./markdown.tsx";
import {
  clearMarkdownNodeCache,
  renderCachedMarkdown,
} from "./markdown/nodeCache.ts";

function render(content) {
  clearMarkdownNodeCache();
  return renderToStaticMarkup(
    renderCachedMarkdown({
      components: createMarkdownComponents(true, false),
      content,
      variant: "ol-start",
    }),
  );
}

test("a lone 8. keeps start=8 so the thread does not paint 1.", () => {
  const html = render("8.");
  assert.match(html, /<ol[^>]*start="8"/);
});

test("a lone 42. keeps start=42", () => {
  const html = render("42.");
  assert.match(html, /<ol[^>]*start="42"/);
});

test("8 without a period is a paragraph, not a list", () => {
  const html = render("8");
  assert.doesNotMatch(html, /<ol\b/);
  assert.match(html, />8</);
});
