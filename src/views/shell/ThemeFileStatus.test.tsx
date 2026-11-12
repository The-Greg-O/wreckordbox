/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";

import { followThemeFrom, type Theme, type ThemeProblem, type ThemeRoles } from "@/lib/themeRoles";
import { ThemeFileStatus } from "./ThemeFileStatus";

const TOKYO: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};

let root: Root;
let host: HTMLDivElement;
let announce: (theme: Theme) => void;

beforeEach(async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await followThemeFrom({
    current: () => Promise.resolve({ roles: null, font: null }),
    onChange: (listener) => { announce = listener; return Promise.resolve(); },
  }, { style: { setProperty: () => {}, removeProperty: () => {} }, setAttribute: () => {}, removeAttribute: () => {} });
  act(() => root.render(<ThemeFileStatus />));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const show = (problems: ThemeProblem[], roles: ThemeRoles = TOKYO) => act(() => announce({ roles, font: null, problems }));
const notice = () => host.querySelector('[role="status"][aria-label="Theme file"]');
const TYPO = [{ line: 2, message: 'unknown key "acent"; it is ignored' }];

it("shows each mistake with its line", () => {
  expect(notice()).toBeNull();
  show([...TYPO, { line: 5, message: "muted is \"grey\", not a colour" }]);
  expect(notice()?.textContent).toContain("line 2");
  expect(notice()?.textContent).toContain('unknown key "acent"');
  expect(notice()?.getAttribute("title")).toContain("line 5");
});

it("stays dismissed while the same mistakes come with other colours, and returns when they change", () => {
  show(TYPO);
  act(() => (notice()?.querySelector("button") as HTMLButtonElement).click());
  expect(notice()).toBeNull();
  // A valid colour saved (or an Omarchy switch): a new event, the same mistakes.
  show([...TYPO], { ...TOKYO, accent: "#ff8800" });
  expect(notice()).toBeNull();
  // A different mistake.
  show([{ line: 3, message: "follow is \"gnome\"" }]);
  expect(notice()?.textContent).toContain("line 3");
});

it("forgets a dismissal once the file is fixed, so the same mistake made again is shown", () => {
  show(TYPO);
  act(() => (notice()?.querySelector("button") as HTMLButtonElement).click());
  show([]);
  expect(notice()).toBeNull();
  show(TYPO);
  expect(notice()?.textContent).toContain("line 2");
});
