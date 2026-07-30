import { describe, expect, it } from "vitest";
import { orderRefs, type Chip } from "./reforder.ts";

// A commit carrying one of every kind, in the backend's own delivered order
// (tag -> head -> branch -> remote, per git_read.rs::collect_refs).
const MIXED: Chip[] = [
  { label: "v1.2.0", kind: "tag" },
  { label: "main", kind: "head" },
  { label: "feature", kind: "branch" },
  { label: "origin/main", kind: "remote" },
];
const labels = (cs: Chip[]) => cs.map((c) => c.label);

describe("orderRefs", () => {
  it("returns [] for empty/nullish input without throwing", () => {
    expect(orderRefs([], true)).toEqual([]);
    expect(orderRefs(null, true)).toEqual([]);
    expect(orderRefs(undefined, false, 3)).toEqual([]);
  });

  it("tagsFirst keeps the backend order (tag, head, branch, remote)", () => {
    expect(labels(orderRefs(MIXED, true))).toEqual(["v1.2.0", "main", "feature", "origin/main"]);
  });

  it("branch-first promotes head + local branches ahead of tags, remotes still last", () => {
    expect(labels(orderRefs(MIXED, false))).toEqual(["main", "feature", "v1.2.0", "origin/main"]);
  });

  it("rotates left by rot AFTER the priority sort", () => {
    expect(labels(orderRefs(MIXED, true, 1))).toEqual(["main", "feature", "origin/main", "v1.2.0"]);
    expect(labels(orderRefs(MIXED, true, 2))).toEqual(["feature", "origin/main", "v1.2.0", "main"]);
  });

  it("wraps rotation around the length (and handles negatives)", () => {
    // rot === length is a full turn back to the start.
    expect(labels(orderRefs(MIXED, true, 4))).toEqual(labels(orderRefs(MIXED, true, 0)));
    expect(labels(orderRefs(MIXED, true, 5))).toEqual(labels(orderRefs(MIXED, true, 1)));
    expect(labels(orderRefs(MIXED, true, -1))).toEqual(labels(orderRefs(MIXED, true, 3)));
  });

  it("is a stable sort — two tags keep their incoming relative order", () => {
    const twoTags: Chip[] = [
      { label: "v2.0", kind: "tag" },
      { label: "v1.9", kind: "tag" },
      { label: "main", kind: "head" },
    ];
    expect(labels(orderRefs(twoTags, false))).toEqual(["main", "v2.0", "v1.9"]);
  });

  it("does not mutate the input array", () => {
    const before = labels(MIXED);
    orderRefs(MIXED, false, 2);
    expect(labels(MIXED)).toEqual(before);
  });

  it("treats an unknown kind as lowest priority (trails known kinds)", () => {
    const withMystery: Chip[] = [
      { label: "weird", kind: "note" },
      { label: "main", kind: "head" },
    ];
    expect(labels(orderRefs(withMystery, true))).toEqual(["main", "weird"]);
    expect(labels(orderRefs(withMystery, false))).toEqual(["main", "weird"]);
  });
});
