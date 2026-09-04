/**
 * Preview origin shared by the desktop Playwright configuration and tests.
 *
 * The default remains 4173. Set BUZZ_E2E_PORT when multiple checkouts run E2E
 * tests concurrently so each checkout serves and exercises its own dist tree.
 */
export const PREVIEW_PORT = process.env.BUZZ_E2E_PORT ?? "4173";

export const PREVIEW_ORIGIN = `http://127.0.0.1:${PREVIEW_PORT}`;
