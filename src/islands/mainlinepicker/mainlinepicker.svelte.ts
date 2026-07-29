// Mainline-parent chooser for cherry-picking a MERGE commit — controller
// (Svelte 5 runes singleton).
//
// git refuses to cherry-pick a merge commit without `-m <n>`, because a merge
// has two parents and git needs to know which side's changes to bring in.
// resolver.startPick calls merge_parents first; when it comes back with >= 2
// parents, it awaits choose() here, which opens a small modal and resolves with
// the 1-based parent number the user picked (or null if they cancel).
//
// Promise-based (like a confirm dialog) rather than event-driven so the caller
// reads as a straight `const n = await mainlinePickerCtrl.choose(...)`.

import type { MergeParent } from "../../ipc/bindings";

class MainlinePickerState {
  open = $state(false);
  sha = $state("");
  parents = $state<MergeParent[]>([]);
  // Resolver for the in-flight choose() promise; null when no chooser is open.
  private resolveFn: ((n: number | null) => void) | null = null;

  // Show the chooser for `sha`'s parents and resolve with the picked 1-based
  // mainline number, or null if cancelled (Esc / Cancel / a second open).
  choose(sha: string, parents: MergeParent[]): Promise<number | null> {
    this.settle(null); // never leave a prior chooser hanging
    this.sha = sha;
    this.parents = parents;
    this.open = true;
    return new Promise((resolve) => {
      this.resolveFn = resolve;
    });
  }

  pick(n: number): void {
    this.settle(n);
  }

  cancel(): void {
    this.settle(null);
  }

  private settle(n: number | null): void {
    this.open = false;
    const r = this.resolveFn;
    this.resolveFn = null;
    r?.(n);
  }
}

export const mainlinePickerCtrl = new MainlinePickerState();
