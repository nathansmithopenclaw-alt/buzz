import { expect, type Locator, type Page } from "@playwright/test";

/**
 * Wait for finishing CSS/Web animations on the page to complete, bounded by a
 * short timeout.
 *
 * Radix UI components animate in via CSS transitions. Playwright's
 * `toBeVisible()` resolves mid-animation, producing greyed-out or
 * partially-rendered screenshots. Call this before any `page.screenshot()`
 * or `locator.screenshot()` to guarantee a fully-rendered frame.
 *
 * Looping animations (spinners, pulsing presence dots) never settle, and
 * animations can also start or restart between sampling and awaiting their
 * `.finished` promise — so an unbounded `Promise.all` can hang until
 * Playwright aborts the `evaluate`. Race the settle against a short ceiling:
 * once the in-flight transitions have had time to land, take the frame.
 */
export async function waitForAnimations(
  page: Page,
  timeoutMs = 1000,
): Promise<void> {
  await page.evaluate((ceiling) => {
    const settled = Promise.all(
      document.getAnimations().map((a) => a.finished.catch(() => undefined)),
    );
    const ceilingHit = new Promise<void>((resolve) =>
      setTimeout(resolve, ceiling),
    );
    return Promise.race([settled.then(() => undefined), ceilingHit]);
  }, timeoutMs);
}

/** Wait for layout, including Motion's JS-driven springs, to stop moving. */
export async function waitForStableLayout(locator: Locator): Promise<void> {
  let previous: number[] | null = null;
  let stableSince = 0;
  await expect
    .poll(
      async () => {
        const box = await locator.boundingBox();
        if (!box) {
          previous = null;
          return false;
        }
        const current = [box.x, box.y, box.width, box.height];
        const now = Date.now();
        const prior = previous;
        if (
          !prior ||
          current.some((value, i) => Math.abs(value - prior[i]) > 0.01)
        ) {
          stableSince = now;
        }
        previous = current;
        return now - stableSince >= 100;
      },
      {
        message: "element layout settles before interaction/measurement",
        intervals: [50],
      },
    )
    .toBe(true);
}
