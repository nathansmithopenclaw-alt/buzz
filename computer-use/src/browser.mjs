import { InputRefused } from "./refusal.mjs";
import { randomUUID } from "node:crypto";
import { chromium } from "playwright";
import { existsSync } from "node:fs";

const MAX_TEXT = 24_000;
const MAX_REFS = 200;
const VIEWPORT = { width: 1280, height: 800 };

function httpUrl(value) {
  const url = new URL(value);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password
  ) {
    throw new Error(
      "Only HTTP(S) URLs without embedded credentials are supported",
    );
  }
  return url.href;
}

/** Host-local browser owner. Each session has a fresh storage context and input lease. */
export class BrowserService {
  constructor({
    maxSessions = 4,
    idleMs = 15 * 60_000,
    lifetimeMs = 2 * 60 * 60_000,
    executablePath,
    slots,
    profiles,
    now = Date.now,
  } = {}) {
    this.sessions = new Map();
    this.maxSessions = maxSessions;
    this.idleMs = idleMs;
    this.lifetimeMs = lifetimeMs;
    this.executablePath = executablePath;
    this.now = now;
    this.browser = null;
    this.launching = null;
    this.closed = false;
    this.pending = 0;
    this.slots = slots;
    this.profiles = profiles;
    this.stopping = null;
  }

  available() {
    return existsSync(this.executablePath || chromium.executablePath());
  }

  async engine() {
    await this.stopping;
    if (this.closed) throw new Error("Browser service is closed");
    if (!this.launching) {
      this.launching = chromium
        .launch({
          headless: true,
          executablePath: this.executablePath,
          chromiumSandbox: true,
        })
        .then((browser) => {
          this.browser = browser;
          browser.on("disconnected", () => {
            if (this.browser !== browser) return;
            for (const s of this.sessions.values()) {
              s.closed = true;
              void s.profile
                ?.release()
                .catch((error) =>
                  console.error("Profile cleanup failed:", error.message),
                );
              void s
                .release?.()
                .catch((error) =>
                  console.error(
                    "Browser reservation cleanup failed:",
                    error.message,
                  ),
                );
            }
            this.sessions.clear();
            this.browser = null;
            this.launching = null;
          });
          return browser;
        })
        .catch((error) => {
          this.launching = null;
          throw error;
        });
    }
    return this.launching;
  }

  async open(url = "about:blank", profileName) {
    if (url !== "about:blank") httpUrl(url);
    if (this.sessions.size + this.pending >= this.maxSessions)
      throw new Error("Browser session limit reached; close a session first");
    this.pending++;
    let context;
    let release;
    let profile;
    try {
      release = await this.slots?.acquire();
      if (profileName) {
        if (!this.profiles)
          throw new Error("Persistent profiles are unavailable");
        profile = await this.profiles.acquire(profileName);
      }
      const browser = await this.engine();
      context = await browser.newContext({
        viewport: VIEWPORT,
        deviceScaleFactor: 1,
        acceptDownloads: false,
        serviceWorkers: "block",
        storageState: profile?.storageState,
      });
      context.setDefaultTimeout(10_000);
      context.setDefaultNavigationTimeout(20_000);
      const page = await context.newPage();
      const session = {
        id: randomUUID(),
        context,
        page,
        controller: "agent",
        generation: 1,
        created: this.now(),
        touched: this.now(),
        queue: Promise.resolve(),
        refs: [],
        observation: null,
        dialog: null,
        diagnostics: [],
        closed: false,
        release,
        profile,
      };
      // Extra windows must not silently create unowned pages in this first slice.
      context.on("page", (popup) => {
        if (popup !== page) void popup.close().catch(() => {});
      });
      page.on("dialog", (dialog) => {
        session.dialog = dialog;
      });
      page.on("framenavigated", (frame) => {
        if (frame === page.mainFrame()) {
          session.observation = null;
          session.generation++;
        }
      });
      page.on("console", (message) => {
        session.diagnostics.push({
          type: message.type(),
          text: message.text().slice(0, 2000),
        });
        if (session.diagnostics.length > 100) session.diagnostics.shift();
      });
      page.on("close", () => {
        session.closed = true;
      });
      if (url !== "about:blank")
        await page.goto(url, { waitUntil: "domcontentloaded" });
      if (this.closed)
        throw new Error("Browser service closed during session creation");
      this.sessions.set(session.id, session);
      return this.info(session);
    } catch (error) {
      await context?.close();
      await release?.();
      await profile?.release();
      throw error;
    } finally {
      this.pending--;
      await this.stopIfEmpty();
    }
  }

  get(id) {
    const session = this.sessions.get(id);
    if (!session || session.closed)
      throw new Error("Browser session is closed or unknown");
    return session;
  }

  info(s) {
    return {
      id: s.id,
      url: s.page.url(),
      controller: s.controller,
      generation: s.generation,
      created: s.created,
      touched: s.touched,
      viewport: s.page.viewportSize(),
      profile: s.profile?.name ?? null,
      observedAt: new Date(this.now()).toISOString(),
      dialog: s.dialog
        ? { type: s.dialog.type(), message: s.dialog.message().slice(0, 2000) }
        : null,
    };
  }

  list() {
    return [...this.sessions.values()]
      .filter((s) => !s.closed)
      .map((s) => this.info(s));
  }

  serial(s, work) {
    const result = s.queue.then(async () => {
      if (s.closed) throw new Error("Browser session is closed");
      s.touched = this.now();
      return work();
    });
    s.queue = result.catch(() => {});
    return result;
  }

  async observe(id) {
    const s = this.get(id);
    return this.serial(s, async () => {
      const generation = s.generation;
      await Promise.all(s.refs.map((ref) => ref.dispose()));
      const refs = await s.page.$$(
        "a[href],button,input,textarea,select,[role=button],[contenteditable=true]",
      );
      s.refs = refs.slice(0, MAX_REFS);
      await Promise.all(refs.slice(MAX_REFS).map((ref) => ref.dispose()));
      const elements = await Promise.all(
        s.refs.map(async (ref, index) => ({
          ref: index,
          ...(await ref.evaluate((el) => ({
            tag: el.tagName.toLowerCase(),
            role: el.getAttribute("role"),
            name: (
              el.getAttribute("aria-label") ||
              el.labels?.[0]?.innerText ||
              el.innerText ||
              el.getAttribute("placeholder") ||
              ""
            ).slice(0, 200),
            type: el.getAttribute("type"),
            value:
              el.type === "password"
                ? "[redacted]"
                : String(el.value ?? "").slice(0, 500),
          }))),
        })),
      );
      const result = {
        ...this.info(s),
        title: await s.page.title(),
        text: (await s.page.locator("body").innerText()).slice(0, MAX_TEXT),
        elements,
        image: await s.page.screenshot({
          type: "jpeg",
          quality: 75,
          animations: "disabled",
        }),
      };
      if (generation !== s.generation)
        throw new Error(
          "Page or control changed during observation; observe again",
        );
      s.observation = randomUUID();
      return { ...result, observation: s.observation };
    });
  }

  async frame(id) {
    const s = this.get(id);
    return {
      ...this.info(s),
      image: await s.page.screenshot({
        type: "jpeg",
        quality: 65,
        timeout: 5000,
      }),
    };
  }

  async control(id, controller) {
    if (!["human", "agent"].includes(controller))
      throw new Error("Invalid controller");
    const s = this.get(id);
    // Fence queued agent actions immediately; acknowledge takeover only after the in-flight action finishes.
    s.controller = controller;
    s.generation++;
    s.observation = null;
    await s.queue;
    return this.info(s);
  }

  async act(id, action, actor = "agent") {
    const s = this.get(id);
    return this.serial(s, async () => {
      if (actor !== s.controller)
        throw new InputRefused(
          `Input is owned by ${s.controller}; wait for explicit handback`,
        );
      if (action.generation !== s.generation)
        throw new InputRefused(
          "Stale page/control generation; no input sent. Call browser_observe, then copy its generation exactly into browser_action. Never increment or guess generation: observing does not advance it.",
        );
      let ref;
      if (action.ref !== undefined) {
        if (!s.observation || action.observation !== s.observation)
          throw new InputRefused(
            "Stale element observation; no input sent. Call browser_observe, then include its observation token together with the ref and generation in browser_action. Every action consumes the observation.",
          );
        ref = s.refs[action.ref];
        if (!ref) throw new InputRefused("Unknown element reference");
      }
      const viewport = s.page.viewportSize();
      switch (action.kind) {
        case "resize":
          if (
            ![action.width, action.height].every(Number.isInteger) ||
            action.width < 320 ||
            action.width > 1920 ||
            action.height < 200 ||
            action.height > 1200
          )
            throw new InputRefused(
              "Viewport must be 320–1920 by 200–1200 pixels",
            );
          await s.page.setViewportSize({
            width: action.width,
            height: action.height,
          });
          s.generation++;
          break;
        case "navigate":
          await s.page.goto(httpUrl(action.url), {
            waitUntil: "domcontentloaded",
          });
          break;
        case "back":
          await s.page.goBack({ waitUntil: "domcontentloaded" });
          break;
        case "forward":
          await s.page.goForward({ waitUntil: "domcontentloaded" });
          break;
        case "reload":
          await s.page.reload({ waitUntil: "domcontentloaded" });
          break;
        case "click":
          if (ref) await ref.click();
          else {
            if (
              !Number.isFinite(action.x) ||
              !Number.isFinite(action.y) ||
              action.x < 0 ||
              action.x >= viewport.width ||
              action.y < 0 ||
              action.y >= viewport.height
            )
              throw new InputRefused("Coordinates outside viewport");
            await s.page.mouse.click(action.x, action.y);
          }
          break;
        case "fill":
          if (typeof action.text !== "string")
            throw new InputRefused("fill requires text");
          if (!ref)
            throw new InputRefused(
              "fill requires an observed element reference",
            );
          await ref.fill(String(action.text).slice(0, MAX_TEXT));
          break;
        case "type":
          if (typeof action.text !== "string")
            throw new InputRefused("type requires text");
          await s.page.keyboard.insertText(
            String(action.text).slice(0, MAX_TEXT),
          );
          break;
        case "key":
          await s.page.keyboard.press(String(action.key).slice(0, 100));
          break;
        case "scroll": {
          if (!Number.isFinite(action.dy) || Math.abs(action.dy) > 4000)
            throw new InputRefused("Invalid scroll distance");
          await s.page.mouse.wheel(0, action.dy);
          break;
        }
        case "dialog":
          if (!s.dialog) throw new InputRefused("No pending dialog");
          if (action.accept === true) await s.dialog.accept(action.text);
          else await s.dialog.dismiss();
          s.dialog = null;
          break;
        default:
          throw new InputRefused("Unsupported browser action");
      }
      s.observation = null;
      return this.info(s);
    });
  }

  async close(id) {
    const s = this.sessions.get(id);
    if (!s) return;
    s.closed = true;
    this.sessions.delete(id);
    try {
      if (s.profile) await s.profile.save(s.context);
    } finally {
      await s.context.close();
      await s.queue;
      await Promise.all([s.release?.(), s.profile?.release()]);
      await this.stopIfEmpty();
    }
  }

  async stopIfEmpty() {
    if (this.sessions.size || this.pending || !this.launching) return;
    this.stopping ??= (async () => {
      const browser = await this.launching;
      await browser.close();
    })();
    try {
      await this.stopping;
    } finally {
      this.stopping = null;
    }
  }

  async reap() {
    const now = this.now();
    for (const s of this.sessions.values()) {
      if (
        s.closed ||
        now - s.touched >= this.idleMs ||
        now - s.created >= this.lifetimeMs
      )
        await this.close(s.id);
    }
  }

  async shutdown() {
    this.closed = true;
    await Promise.all([...this.sessions.keys()].map((id) => this.close(id)));
    // A launch in flight must also be joined, otherwise shutdown can leave a late browser behind.
    const browser = await this.launching?.catch(() => null);
    await browser?.close();
    this.browser = null;
    this.launching = null;
  }
}
