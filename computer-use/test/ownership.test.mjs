import { assertDetachedBrowser } from "../../scripts/require-detached-browser.mjs";
assertDetachedBrowser();

import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { HostSlots } from "../src/slots.mjs";
import { BrowserService } from "../src/browser.mjs";
import { startViewer } from "../src/viewer.mjs";
import { Profiles } from "../src/profiles.mjs";
import { createServer } from "node:http";
import { chromium } from "playwright";

test("independent services share admission and release engine capacity", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "buzz-slots-"));
  const a = new BrowserService({ slots: new HostSlots(directory, 1) });
  const b = new BrowserService({ slots: new HostSlots(directory, 1) });
  t.after(async () => {
    await Promise.all([a.shutdown(), b.shutdown()]);
    await rm(directory, { recursive: true });
  });
  const opened = await a.open();
  await assert.rejects(b.open(), /Host browser capacity/);
  await a.close(opened.id);
  assert.equal(a.browser, null);
  await b.open();
  assert.equal(b.list().length, 1);
});

test("dead owner is reclaimed once while a live owner cannot be stolen", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "buzz-dead-slot-"));
  t.after(() => rm(directory, { recursive: true }));
  await mkdir(join(directory, "0"));
  await writeFile(
    join(directory, "0", "old.json"),
    JSON.stringify({ pid: 2147483647 }),
  );
  const slots = new HostSlots(directory, 1);
  const results = await Promise.allSettled([slots.acquire(), slots.acquire()]);
  assert.equal(results.filter((r) => r.status === "fulfilled").length, 1);
  await assert.rejects(slots.acquire(), /capacity/);
  await results.find((r) => r.status === "fulfilled").value();
  await (await slots.acquire())();
});

test("human viewer buttons and keyboard operate the same page and return ownership", async (t) => {
  const service = new BrowserService();
  const viewer = await startViewer(service);
  const browser = await chromium.launch();
  t.after(async () => {
    await browser.close();
    await viewer.close();
    await service.shutdown();
  });
  const session = await service.open();
  await service.get(session.id).page.setContent("<label>Name <input></label>");
  const ui = await browser.newPage();
  await ui.goto(viewer.url);
  await ui.getByRole("button", { name: "Take control", exact: true }).click();
  await ui
    .getByRole("button", { name: "Return to agent", exact: true })
    .waitFor();
  assert.equal(service.get(session.id).controller, "human");
  await service.get(session.id).page.waitForFunction(() => innerHeight < 800);
  const target = await service
    .get(session.id)
    .page.locator("input")
    .boundingBox();
  const viewport = service.get(session.id).page.viewportSize();
  await ui.waitForFunction(
    (height) => document.getElementById("page").naturalHeight === height,
    viewport.height,
  );
  const image = ui.getByRole("img");
  await image.waitFor({ state: "visible" });
  const shown = await image.boundingBox();
  await image.click({
    position: {
      x: ((target.x + 10) * shown.width) / viewport.width,
      y: ((target.y + 10) * shown.height) / viewport.height,
    },
  });
  await ui
    .getByRole("textbox", { name: "Text to enter" })
    .fill("human through UI");
  await ui.getByRole("button", { name: "Type text", exact: true }).click();
  await service
    .get(session.id)
    .page.waitForFunction(
      () => document.querySelector("input").value === "human through UI",
    );
  await ui
    .getByRole("button", { name: "Return to agent", exact: true })
    .click();
  await ui.getByRole("button", { name: "Take control", exact: true }).waitFor();
  assert.equal(service.get(session.id).controller, "agent");
  assert.equal(
    (await service.observe(session.id)).elements[0].value,
    "human through UI",
  );
});

test("opt-in profiles persist only for their owner and reject concurrent writers", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "buzz-profiles-"));
  const host = createServer((_req, res) =>
    res.end("<body>profile fixture</body>"),
  );
  await new Promise((r) => host.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${host.address().port}`;
  const a = new BrowserService({
    profiles: new Profiles(directory, "community/agent-a"),
  });
  const b = new BrowserService({
    profiles: new Profiles(directory, "community/agent-b"),
  });
  t.after(async () => {
    await Promise.all([a.shutdown(), b.shutdown()]);
    host.closeAllConnections();
    await new Promise((r) => host.close(r));
    await rm(directory, { recursive: true });
  });
  const first = await a.open(url, "work");
  await a.get(first.id).page.evaluate(() => {
    localStorage.token = "owner-a";
    // biome-ignore lint/suspicious/noDocumentCookie: verify ordinary site cookie persistence
    document.cookie = "auth=owner-a";
  });
  await assert.rejects(a.open(url, "work"), /profile writer capacity/);
  await a.close(first.id);
  const restored = await a.open(url, "work");
  const independent = await b.open(url, "work");
  assert.equal(
    await a.get(restored.id).page.evaluate(() => localStorage.token),
    "owner-a",
  );
  assert.equal(
    await b.get(independent.id).page.evaluate(() => localStorage.token),
    undefined,
  );
  await assert.rejects(a.open(url, "../escape"), /Profile name/);
});
