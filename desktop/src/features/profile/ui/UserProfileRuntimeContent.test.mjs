import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const panelSectionsSource = await readFile(
  new URL("./UserProfilePanelSections.tsx", import.meta.url),
  "utf8",
);
const panelTabsSource = await readFile(
  new URL("./UserProfilePanelTabs.tsx", import.meta.url),
  "utf8",
);
const previewSource = await readFile(
  new URL("./UserProfileRuntimePreview.tsx", import.meta.url),
  "utf8",
);

test("development profiles union preview data into the runtime tab", () => {
  for (const requiredPattern of [
    /UserProfileRuntimePreview/,
    /fillRuntimePreviewFields/,
    /fillRuntimePreviewDiagnostics/,
    /showRuntimePreview/,
    /UserProfileConfigPreview/,
    /import\.meta\.env\.DEV/,
    /showRuntimeTab[\s\S]*showRuntimePreview/,
  ]) {
    assert.match(panelSectionsSource, requiredPattern);
  }
  assert.match(previewSource, /PROFILE_RUNTIME_PREVIEW/);
  assert.match(previewSource, /Start on launch/);
});

test("preview runtime rows retain the local harness affordances", () => {
  for (const requiredPattern of [
    /previewStartOnLaunchEnabled/,
    /isRuntimePreview/,
    /showPreviewHarnessLog/,
    /diagnostics-ingress-preview/,
  ]) {
    assert.match(panelTabsSource, requiredPattern);
  }
});
