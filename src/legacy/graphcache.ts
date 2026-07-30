// A tiny LRU (least-recently-used) map, extracted from legacy/main.ts (which has
// no unit tests — it boots the whole canvas app on import) so the eviction policy
// can actually be tested. main.ts uses one instance to keep a handful of
// fully-loaded repo graphs in memory: switching back to a repo you were just on
// then restores instantly instead of re-streaming its whole history. Values are
// opaque here (the graph snapshot object) — this owns ONLY which keys to keep.
//
// Recency is tracked by Map insertion order: `set` and a hitting `get` both move
// the key to the newest position, so the key evicted when we exceed `cap` is
// always the one untouched for longest.
export class LruCache<V> {
  private map = new Map<string, V>();
  constructor(private cap: number) {
    // A non-positive cap would evict everything immediately — clamp to at least 1
    // so a misconfigured cap can't silently disable the cache.
    this.cap = Math.max(1, Math.floor(cap));
  }

  has(key: string): boolean {
    return this.map.has(key);
  }

  // Return the value AND mark it most-recently-used, so a repo you keep coming
  // back to survives eviction over ones you opened once and left.
  get(key: string): V | undefined {
    if (!this.map.has(key)) return undefined;
    const v = this.map.get(key)!;
    this.map.delete(key);
    this.map.set(key, v); // re-insert at the newest end
    return v;
  }

  set(key: string, val: V): void {
    if (this.map.has(key)) this.map.delete(key); // move an existing key to newest
    this.map.set(key, val);
    while (this.map.size > this.cap) {
      const oldest = this.map.keys().next().value as string; // first = least-recently-used
      this.map.delete(oldest);
    }
  }

  delete(key: string): boolean {
    return this.map.delete(key);
  }

  clear(): void {
    this.map.clear();
  }

  get size(): number {
    return this.map.size;
  }

  // Oldest → newest, for tests/inspection.
  keys(): string[] {
    return [...this.map.keys()];
  }
}
