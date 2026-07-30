// Submodule NAVIGATOR — the slim strip under the topbar (+ its full-tree
// popover) for moving between a superproject and its submodules WITHOUT the old
// parent-only "← Back" button. It's purely a switch-between-repos surface; every
// submodule MUTATION (init/update/sync/add/remove) still lives in the sidebar's
// Submodules section.
//
// The whole thing rides on ONE primitive in legacy/main.ts —
// `navigateToRepo(absolutePath, chain)` — which sets NAV_STACK to `chain` and
// re-opens the app at `absolutePath` (openRepo, same as every other "open a
// repository" path). Enter a child, hop to a sibling, jump to any ancestor or an
// arbitrary tree node: all of it is just "open this repo with THAT ancestor
// chain", so a single call covers the lot. legacy/main.ts's enterSubmodule /
// goBackToParent are now thin wrappers over it, and it refreshes THIS controller
// on success — so the strip always mirrors wherever the app actually is.
//
// Reads bridge.NAV_STACK (ancestors, root..immediate-parent) and bridge.CUR_REPO
// (the open repo) — both live bindings — and asks the backend for one level of
// `submodule_status` at a time (the current level for the sibling tabs; walked
// recursively, lazily on open, for the tree).
import * as bridge from "../../legacy/bridge";
import { commands } from "../../ipc/bindings";
import { IN_TAURI } from "../../ipc/env";
import { submoduleCanOpen } from "../sidebar/sidebar.svelte.ts";
import type { SubmoduleInfo } from "../../ipc/bindings";

function basename(p: string): string {
  return p.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || p;
}

// One clickable step in the "root › vendor/lib-a › nested" path. `chain` is the
// ancestor list (root..parent) to hand navigateToRepo when jumping HERE.
export interface RepoCrumb {
  name: string;
  absolutePath: string;
  chain: string[];
  current: boolean;
}
// One sibling tab at the current level (or, at the top, the superproject's own
// submodules to dive into). All siblings share the same parent, hence one
// `chain` for the row — stored per-tab so a test can assert the jump target.
export interface SiblingTab {
  name: string;
  absolutePath: string;
  status: string;
  canOpen: boolean;
  current: boolean;
  chain: string[];
}
// A node in the full-tree popover. `chain` is root..parent for THIS node; the
// synthetic root has chain [] (jump to root = navigateToRepo(root, [])).
export interface TreeNode {
  name: string;
  absolutePath: string;
  status: string; // "" for the synthetic superproject root
  chain: string[];
  canOpen: boolean;
  current: boolean;
  isRoot: boolean;
  children: TreeNode[];
}

const TREE_MAX_DEPTH = 8; // matches the backend's own recursion guard; also caps a pathological tree

// A tiny stand-in so the browser design-mode preview (IN_TAURI === false) shows
// a representative strip instead of an empty bar — mirrors the sidebar's own
// DEMO_SUBMODULES idea without importing its (private) list.
const DEMO_SIBLINGS: SiblingTab[] = [
  { name: "vendor/lib-a", absolutePath: "/demo/gitcat/vendor/lib-a", status: "clean", canOpen: true, current: false, chain: ["/demo/gitcat"] },
  { name: "vendor/lib-b", absolutePath: "/demo/gitcat/vendor/lib-b", status: "dirty", canOpen: true, current: false, chain: ["/demo/gitcat"] },
  { name: "third_party/tool", absolutePath: "/demo/gitcat/third_party/tool", status: "out-of-date", canOpen: true, current: false, chain: ["/demo/gitcat"] },
];

class SubmoduleNavState {
  path = $state<RepoCrumb[]>([]);
  siblings = $state<SiblingTab[]>([]);
  busy = $state(false);
  busyTarget = $state<string | null>(null);
  // Tree popover.
  treeOpen = $state(false);
  treeLoading = $state(false);
  tree = $state<TreeNode | null>(null);

  // The strip earns its row only when there's something to navigate: you're
  // inside a submodule (path has >1 crumb) OR the current repo has submodules to
  // dive into (siblings non-empty). A plain repo with no submodules → no strip.
  get visible(): boolean {
    return this.path.length > 1 || this.siblings.length > 0;
  }

  private stack(): string[] {
    return (bridge.NAV_STACK as unknown as string[]) || [];
  }
  private cur(): string {
    return (bridge.CUR_REPO as unknown as string) || "";
  }

  // Rebuild the breadcrumb + sibling tabs for wherever the app currently is.
  // Called by navigateToRepo on success and by the repo-reset paths (pickRepo /
  // closeRepo / boot) — never on a timer, so it can't show a stale location.
  async refresh(repo: string): Promise<void> {
    if (!IN_TAURI) {
      // Design-mode preview: a superproject sitting on three demo submodules.
      this.path = [{ name: "gitcat", absolutePath: "/demo/gitcat", chain: [], current: true }];
      this.siblings = DEMO_SIBLINGS;
      return;
    }
    const stack = this.stack().slice();
    // Breadcrumb: each ancestor (jump-to chain = everything before it), then the
    // current repo (jump-to chain = the whole ancestor stack).
    const crumbs: RepoCrumb[] = stack.map((abs, i) => ({
      name: basename(abs),
      absolutePath: abs,
      chain: stack.slice(0, i),
      current: false,
    }));
    crumbs.push({ name: basename(repo), absolutePath: repo, chain: stack.slice(), current: true });
    this.path = crumbs;

    // Sibling tabs: one level of the CURRENT level's parent (root..parent's last,
    // or the repo itself at the top). Their shared parent chain is the current
    // repo's own chain (the ancestor stack), or [repo] at the top level.
    const listFrom = stack.length ? stack[stack.length - 1] : repo;
    const sibChain = stack.length ? stack.slice() : [repo];
    try {
      const res = await commands.submoduleStatus(listFrom);
      const subs: SubmoduleInfo[] = res.status === "ok" ? res.data : [];
      this.siblings = subs.map((s) => ({
        name: s.path,
        absolutePath: s.absolutePath,
        status: s.status,
        canOpen: submoduleCanOpen(s.status),
        current: s.absolutePath === repo,
        chain: sibChain,
      }));
    } catch (e) {
      console.error("submodulenav.refresh", e);
      this.siblings = [];
    }
  }

  // Jump anywhere. `key` scopes the row spinner (and guards double-clicks);
  // jumping to the already-open repo is a no-op. navigateToRepo refreshes this
  // controller itself on success, so there's nothing to reconcile here after.
  async jumpTo(absolutePath: string, chain: string[], key: string): Promise<void> {
    if (this.busy) return;
    if (absolutePath === this.cur()) return; // already here
    if (!IN_TAURI) {
      bridge.tama.set("hint");
      bridge.tama.say("Switched to " + basename(absolutePath) + " (demo).");
      return;
    }
    this.busy = true;
    this.busyTarget = key;
    try {
      await bridge.navigateToRepo(absolutePath, chain);
    } catch (e) {
      console.error("submodulenav.jumpTo", e);
      bridge.tama.warn("Couldn't switch to " + basename(absolutePath));
    } finally {
      this.busy = false;
      this.busyTarget = null;
    }
  }

  jumpToCrumb(i: number): Promise<void> {
    const c = this.path[i];
    if (!c || c.current) return Promise.resolve();
    return this.jumpTo(c.absolutePath, c.chain, "crumb:" + i);
  }

  jumpToSibling(s: SiblingTab): Promise<void> {
    if (s.current || !s.canOpen) return Promise.resolve();
    return this.jumpTo(s.absolutePath, s.chain, "sib:" + s.absolutePath);
  }

  jumpToNode(n: TreeNode): Promise<void> {
    if (n.current) return Promise.resolve();
    if (!n.isRoot && !n.canOpen) return Promise.resolve();
    this.closeTree();
    return this.jumpTo(n.absolutePath, n.chain, "node:" + n.absolutePath);
  }

  // Full-tree popover: built eagerly on open by walking submodule_status from the
  // root superproject (NAV_STACK[0] ?? CUR_REPO). Cheap for the small trees this
  // is for; a visited-set + depth cap keep a cyclic/absurd tree bounded, and only
  // openable nodes are descended into (uninitialised/removed/unreadable have
  // nothing to walk).
  async toggleTree(): Promise<void> {
    if (this.treeOpen) {
      this.closeTree();
      return;
    }
    this.treeOpen = true;
    if (!IN_TAURI) {
      const root = "/demo/gitcat";
      this.tree = {
        name: "gitcat", absolutePath: root, status: "", chain: [], canOpen: true, current: true, isRoot: true,
        children: DEMO_SIBLINGS.map((s) => ({
          name: s.name, absolutePath: s.absolutePath, status: s.status, chain: [root],
          canOpen: s.canOpen, current: false, isRoot: false, children: [],
        })),
      };
      return;
    }
    this.treeLoading = true;
    try {
      const stack = this.stack();
      const root = stack.length ? stack[0] : this.cur();
      const cur = this.cur();
      const children = await this.walk(root, [], new Set([root]), 0, cur);
      this.tree = {
        name: basename(root), absolutePath: root, status: "", chain: [],
        canOpen: true, current: root === cur, isRoot: true, children,
      };
    } catch (e) {
      console.error("submodulenav.toggleTree", e);
      this.tree = null;
    } finally {
      this.treeLoading = false;
    }
  }

  closeTree(): void {
    this.treeOpen = false;
  }

  // No repo open (cold boot / Close Repository): empty everything so the strip's
  // grid row collapses. Mirrors sidebarCtrl.reset()'s role in bootEmpty().
  reset(): void {
    this.path = [];
    this.siblings = [];
    this.busy = false;
    this.busyTarget = null;
    this.treeOpen = false;
    this.tree = null;
  }

  private async walk(absPath: string, chain: string[], visited: Set<string>, depth: number, cur: string): Promise<TreeNode[]> {
    if (depth >= TREE_MAX_DEPTH) return [];
    let subs: SubmoduleInfo[] = [];
    try {
      const res = await commands.submoduleStatus(absPath);
      subs = res.status === "ok" ? res.data : [];
    } catch (e) {
      console.error("submodulenav.walk", absPath, e);
      return [];
    }
    const out: TreeNode[] = [];
    for (const s of subs) {
      const canOpen = submoduleCanOpen(s.status);
      const nodeChain = [...chain, absPath];
      let children: TreeNode[] = [];
      // Descend only into openable, not-yet-visited nodes — a cycle guard AND
      // the natural stop for uninitialised/removed/unreadable rows.
      if (canOpen && !visited.has(s.absolutePath)) {
        visited.add(s.absolutePath);
        children = await this.walk(s.absolutePath, nodeChain, visited, depth + 1, cur);
      }
      out.push({
        name: s.path, absolutePath: s.absolutePath, status: s.status, chain: nodeChain,
        canOpen, current: s.absolutePath === cur, isRoot: false, children,
      });
    }
    return out;
  }
}

export const submoduleNavCtrl = new SubmoduleNavState();
