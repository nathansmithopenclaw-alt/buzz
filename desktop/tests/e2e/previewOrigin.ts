/**
 * The origin the e2e preview server is served from.
 *
 * The port stays 4173 by default, so a normal run and CI are unchanged. Set
 * `BUZZ_E2E_PORT` to move it, which is what makes it safe to run two checkouts
 * at once:
 *
 *   BUZZ_E2E_PORT=4319 pnpm exec playwright test --project=integration
 *
 * Both Playwright configs reuse an already-listening server (`playwright.config.ts`
 * outside CI, `playwright.perf.config.ts` always). The web server is
 * `python3 -m http.server <port> -d dist`, resolved against whichever checkout
 * started it — so two worktrees sharing 4173 do not merely race, they silently
 * serve each other's `dist/` and report passes and failures that belong to the
 * other checkout's build.
 */
export const PREVIEW_PORT = process.env.BUZZ_E2E_PORT ?? "4173";

export const PREVIEW_ORIGIN = `http://127.0.0.1:${PREVIEW_PORT}`;
