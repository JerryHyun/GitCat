import { describe, expect, it } from "vitest";
import { LruCache } from "./graphcache.ts";

describe("LruCache", () => {
  it("stores and retrieves values, and reports has/size", () => {
    const c = new LruCache<number>(3);
    expect(c.has("a")).toBe(false);
    c.set("a", 1);
    expect(c.get("a")).toBe(1);
    expect(c.has("a")).toBe(true);
    expect(c.size).toBe(1);
    expect(c.get("missing")).toBeUndefined();
  });

  it("evicts the least-recently-USED key once over capacity", () => {
    const c = new LruCache<number>(2);
    c.set("a", 1);
    c.set("b", 2);
    c.set("c", 3); // over cap → "a" (oldest) evicted
    expect(c.has("a")).toBe(false);
    expect(c.keys()).toEqual(["b", "c"]);
    expect(c.size).toBe(2);
  });

  it("a get() rescues a key from eviction by marking it most-recently-used", () => {
    const c = new LruCache<number>(2);
    c.set("a", 1);
    c.set("b", 2);
    expect(c.get("a")).toBe(1); // "a" is now newest, "b" is oldest
    c.set("c", 3); // over cap → "b" evicted, not "a"
    expect(c.has("a")).toBe(true);
    expect(c.has("b")).toBe(false);
    expect(c.keys()).toEqual(["a", "c"]);
  });

  it("re-setting an existing key updates the value and refreshes recency", () => {
    const c = new LruCache<number>(2);
    c.set("a", 1);
    c.set("b", 2);
    c.set("a", 11); // update + move to newest; "b" now oldest
    expect(c.get("a")).toBe(11);
    expect(c.size).toBe(2);
    c.set("c", 3); // "b" evicted
    expect(c.has("b")).toBe(false);
    expect(c.has("a")).toBe(true);
  });

  it("delete and clear remove entries", () => {
    const c = new LruCache<number>(3);
    c.set("a", 1);
    c.set("b", 2);
    expect(c.delete("a")).toBe(true);
    expect(c.delete("a")).toBe(false);
    expect(c.has("a")).toBe(false);
    c.clear();
    expect(c.size).toBe(0);
    expect(c.has("b")).toBe(false);
  });

  it("clamps a non-positive capacity to 1 instead of evicting everything", () => {
    const c = new LruCache<number>(0);
    c.set("a", 1);
    expect(c.get("a")).toBe(1);
    c.set("b", 2); // still holds the most recent one
    expect(c.get("b")).toBe(2);
    expect(c.has("a")).toBe(false);
    expect(c.size).toBe(1);
  });
});
