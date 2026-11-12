/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { RowDto } from "@/ipc/types";
import { LIBRARY_LOOK } from "./libraryLook";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const row = (fields: Partial<RowDto>): RowDto => ({
  id: "1", trackNo: 1, title: "T", artist: "", album: "", genre: "", label: "", comment: "",
  bpmX100: 12_000, key: "", durationSec: 300, rating: 0, analysed: 1, dateAdded: "", releaseDate: "",
  hotCues: [], artworkHue: 0, hasArtwork: false, ...fields,
});

function renderCell(r: RowDto) {
  act(() => root.render(<>{LIBRARY_LOOK.cell?.("myTag", r, undefined)}</>));
}

it("shows each MyTag as the Library holds it, a comma in its name included", () => {
  renderCell(row({
    myTags: [
      { id: "10", name: "Dark, Driving", category: "Mood" },
      { id: "11", name: "Vocal", category: "Sound" },
    ],
  }));
  const tags = [...host.querySelectorAll("[data-tag]")].map((tag) => tag.textContent);
  expect(tags).toEqual(["Dark, Driving", "Vocal"]);
});

it("keeps two tags of one name in different categories apart", () => {
  const warn = vi.spyOn(console, "error").mockImplementation(() => {});
  renderCell(row({
    myTags: [
      { id: "10", name: "Vocal", category: "Sound" },
      { id: "20", name: "Vocal", category: "Vocals" },
    ],
  }));
  expect(host.querySelectorAll("[data-tag]")).toHaveLength(2);
  expect([...host.querySelectorAll("[data-tag]")].map((tag) => tag.getAttribute("title"))).toEqual(
    ["Sound: Vocal", "Vocals: Vocal"],
  );
  // No duplicate-key warning from React.
  expect(warn).not.toHaveBeenCalled();
  warn.mockRestore();
});

it("never rebuilds tags from the joined names", () => {
  renderCell(row({ extra: { myTag: "Dark, Driving" } }));
  expect(host.querySelectorAll("[data-tag]")).toHaveLength(0);
});
