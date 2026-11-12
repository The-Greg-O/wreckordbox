import type { Page } from "@playwright/test";

/**
 * Tooltips default off; some assertions need the real text a hover shows.
 * Must run before the page navigates — it seeds the preference the same way
 * `grid-edit.spec.ts` does, rather than clicking through Settings each time.
 */
export async function enableTooltips(page: Page): Promise<void> {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({ view: { tooltips: true } })));
}

/**
 * Turns the view shell on (ticket #50): the four-view frame drawn in place of
 * upstream's layout. Must run before the page navigates. It keeps the Library
 * writable, as the suite's own stored preferences do.
 */
export async function enableViewShell(page: Page): Promise<void> {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary: false },
    view: { viewShell: true },
  })));
}
