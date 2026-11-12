// @vitest-environment jsdom
/**
 * The window's half of a theme switch, through the path the app uses: the
 * app shell's `theme:changed` event, carried by Tauri's own event plumbing
 * (its IPC mocked, as there is no app shell here), heard by `followTheme()`
 * and shown by `applyTheme()`. The event carries the roles the app shell's
 * test (src-tauri/tests/omarchy_theme.rs) requires the real watcher to
 * announce for the same palette, so the two tests meet in one fixture.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import switched from "./themeSwitch.fixture.json";

afterEach(async () => {
  const { clearMocks } = await import("@tauri-apps/api/mocks");
  clearMocks();
  document.documentElement.removeAttribute("style");
  document.documentElement.removeAttribute("data-theme-mode");
  vi.resetModules();
});

/** Starts `followTheme()` as `main.tsx` does, inside a mocked app shell with no theme yet. */
async function startInAppShell(): Promise<void> {
  const { mockIPC } = await import("@tauri-apps/api/mocks");
  mockIPC((command) => (command === "theme_roles" ? { roles: null, font: null } : undefined), { shouldMockEvents: true });
  // The module decides at load whether it runs in the app shell.
  vi.resetModules();
  const { followTheme } = await import("./themeRoles");
  followTheme();
  await vi.waitFor(() => expect(document.documentElement.hasAttribute("data-theme-mode")).toBe(false));
  // Let the subscription and the first ask settle.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

const role = (name: string) => document.documentElement.style.getPropertyValue(name);

describe("followTheme in the app shell", () => {
  it("shows the roles the app shell announces for a swapped theme, and drops them when the theme goes", async () => {
    await startInAppShell();
    const { emit } = await import("@tauri-apps/api/event");

    await emit("theme:changed", { roles: switched.roles, font: "JetBrainsMono Nerd Font" });
    await vi.waitFor(() => expect(document.documentElement.getAttribute("data-theme-mode")).toBe("dark"));
    expect(role("--role-accent")).toBe(switched.roles.accent);
    expect(role("--role-on-accent")).toBe(switched.roles.onAccent);
    expect(role("--role-text")).toBe(switched.roles.text);
    expect(role("--role-tab-from")).toBe(switched.roles.tabFrom);

    await emit("theme:changed", { roles: null, font: null });
    await vi.waitFor(() => expect(document.documentElement.hasAttribute("data-theme-mode")).toBe(false));
    expect(role("--role-accent")).toBe("");
  });
});
