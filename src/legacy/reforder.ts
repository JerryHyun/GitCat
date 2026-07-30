// Pure ordering for a commit's ref chips in the graph gutter — extracted from
// legacy/main.ts's canvas code (which has no unit tests: it boots the whole app
// on import) so the priority + rotation logic can actually be tested.
//
// The backend (git_read.rs::collect_refs) already hands us each commit's refs
// stably sorted tag -> head -> branch -> remote. Two knobs sit on top of that:
//
//   * `tagsFirst` — the global "label priority" preference. `true` keeps the
//     backend order (a commit's tag wins the one visible slot on a narrow
//     gutter); `false` promotes the checked-out branch / local branches ahead
//     of tags for people who'd rather see the branch there.
//   * `rot` — a per-commit rotation the user clicks up via the "+N" overflow
//     chip, so any ref that doesn't fit can be spun to the front. It's applied
//     AFTER the priority sort, so cycling walks the displayed order.
//
// Both are display-only; nothing here mutates the input.

export type RefKind = "head" | "branch" | "tag" | "remote" | string;
export interface Chip {
  label: string;
  kind: RefKind;
}

// tag -> head -> branch -> remote (mirrors the backend's own ordering, so
// tagsFirst is effectively a stable no-op re-sort — cheap and idempotent).
const TAG_FIRST: Record<string, number> = { tag: 0, head: 1, branch: 2, remote: 3 };
// head -> branch -> tag -> remote: the current branch and other local branches
// come before tags; remotes still trail.
const BRANCH_FIRST: Record<string, number> = { head: 0, branch: 1, tag: 2, remote: 3 };

// A copy of `refs`, stably reordered by the chosen priority then rotated left by
// `rot` (any integer; negative and out-of-range values wrap). Empty in, empty
// out. Never mutates the argument.
export function orderRefs<T extends Chip>(refs: readonly T[] | null | undefined, tagsFirst: boolean, rot = 0): T[] {
  if (!refs || refs.length === 0) return [];
  const pri = tagsFirst ? TAG_FIRST : BRANCH_FIRST;
  // Stable sort by kind priority: decorate with the original index so equal
  // kinds keep their incoming relative order (two tags stay in backend order).
  const sorted = refs
    .map((r, i) => ({ r, i }))
    .sort((a, b) => (pri[a.r.kind] ?? 9) - (pri[b.r.kind] ?? 9) || a.i - b.i)
    .map((x) => x.r);
  const n = sorted.length;
  const k = ((rot % n) + n) % n; // normalise into [0, n)
  return k === 0 ? sorted : sorted.slice(k).concat(sorted.slice(0, k));
}
