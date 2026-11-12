import { describe, expect, it } from "vitest";

import { liftMyTags } from "./rowMyTags";

const base = { id: "1", title: "T" };

describe("a row page's MyTags, as the backend sends them", () => {
  it("lifts each tag out of the extra fields as itself, a comma in its name included", () => {
    const tags = [
      { id: "10", name: "Dark, Driving", category: "Mood" },
      { id: "11", name: "Vocal", category: "Sound" },
    ];
    const [row] = liftMyTags([{ ...base, extra: { color: 2, myTags: tags } }]);
    expect(row?.myTags).toEqual(tags);
    expect(row?.extra).toEqual({ color: 2 });
  });

  it("drops anything that is not a tag", () => {
    const [row] = liftMyTags([{ ...base, extra: { myTags: [{ id: "1", name: "A", category: "C" }, { id: 2 }, "x", null] } }]);
    expect(row?.myTags).toEqual([{ id: "1", name: "A", category: "C" }]);
  });

  it("leaves a page that did not ask for them as it came", () => {
    const rows = [{ ...base, extra: { myTag: "Dark, Driving" } }, { ...base }];
    expect(liftMyTags(rows)).toEqual(rows);
  });
});
