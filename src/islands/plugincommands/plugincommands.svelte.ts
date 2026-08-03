// Plugin commands in the ⌘K palette (PER-42) — controller (Svelte 5 runes
// singleton).
//
// The backend (commit 75c5708) owns everything about plugins: `list_plugins`
// returns the installed manifests, and `run_plugin_command(pluginId,
// commandId, ctx)` expands the command's placeholder TEMPLATE and shells out.
// This controller is purely the palette-facing seam: it turns the subset of
// plugin commands that ask to appear in the palette into the SAME
// `ActionItem` shape the static ACTIONS array in cmdk.svelte.ts already uses,
// and — critically — its `run` closure ONLY calls the backend command. It
// never sees, evals, or executes the manifest's `run` string itself; that
// string is a shell template the Rust side alone expands (same trust boundary
// as tool_settings.rs's diff/merge `cmd`). This keeps GitCat's AI-agnostic /
// "we only ever run a user-configured external command" contract intact.
//
// Same "peer-island singleton cmdk imports directly" precedent every other
// ⌘K action already establishes (bisectdrawer/reflog/etc). To avoid a runtime
// import cycle (cmdk imports THIS at runtime), the `ActionItem` shape is
// imported type-only — erased at compile, so nothing here dereferences cmdk at
// module-eval time.

import { commands } from "../../ipc/bindings";
import * as bridge from "../../legacy/bridge";
import { IN_TAURI } from "../../ipc/env";
import type { Plugin, PluginContext, PlaceholderCtx } from "../../ipc/bindings";
import type { ActionItem } from "../cmdk/cmdk.svelte.ts";

class PluginCommandsState {
  // The palette-ready actions cmdk's filter() reads alongside its static
  // ACTIONS. Rebuilt whole on every (re)load — never mutated in place.
  actions = $state<ActionItem[]>([]);
  // Lazy-load gate: ensureLoaded() only ever hits the backend once; reload()
  // is the explicit force path (plugin install/enable/disable).
  loaded = $state(false);
  private loading: Promise<void> | null = null;

  // cmdk wires this after constructing its singleton (see cmdk.svelte.ts's
  // tail): a force reload() re-runs the open palette's filter so newly
  // installed/enabled plugin commands show up without reopening ⌘K. Kept as a
  // settable callback (not a direct cmdkCtrl import) specifically to avoid the
  // runtime import cycle the type-only ActionItem import already dodges.
  onActionsChanged: (() => void) | null = null;

  // Lazy + cached. Concurrent callers (e.g. two quick ⌘K opens) share the one
  // in-flight load.
  async ensureLoaded(): Promise<void> {
    if (this.loaded) return;
    if (this.loading) return this.loading;
    const p = this.load();
    this.loading = p;
    try {
      await p;
    } finally {
      // Only clear the slot if it's still OURS — a concurrent reload() may have
      // installed its own load() in the meantime (see reload()).
      if (this.loading === p) this.loading = null;
    }
  }

  // Force a fresh read (plugin registry changed) and notify the palette. Shares
  // the in-flight `loading` slot with ensureLoaded() so a force reload can't run
  // load() concurrently with a lazy one (a last-write race on `actions`): wait
  // for any in-flight load first, then force exactly one fresh one. (load() is
  // infallible — it swallows its own errors — so awaiting it never throws.)
  async reload(): Promise<void> {
    if (this.loading) await this.loading;
    this.loaded = false;
    const p = this.load();
    this.loading = p;
    try {
      await p;
    } finally {
      if (this.loading === p) this.loading = null;
    }
    this.onActionsChanged?.();
  }

  private async load(): Promise<void> {
    // Design mode (plain browser) has no plugin backend — an empty palette is
    // the correct, non-confusing demo state (same discipline as every other
    // island's !IN_TAURI branch).
    if (!IN_TAURI) {
      this.actions = [];
      this.loaded = true;
      return;
    }
    try {
      const res = await commands.listPlugins();
      this.actions = res.status === "ok" ? this.build(res.data) : [];
    } catch {
      // A failed registry read must never break the palette — the static
      // ACTIONS still work; plugin actions simply stay absent this session.
      this.actions = [];
    }
    this.loaded = true;
  }

  // Keep ENABLED plugins (enabled defaults to true when a manifest omits it),
  // and only their commands whose placement reaches the palette ("palette" or
  // "both"; placement defaults to "palette" when omitted).
  private build(plugins: Plugin[]): ActionItem[] {
    const out: ActionItem[] = [];
    for (const p of plugins) {
      if (p.enabled === false) continue;
      for (const c of p.commands ?? []) {
        const placement = c.placement ?? "palette";
        if (placement !== "palette" && placement !== "both") continue;
        out.push({
          type: "action",
          id: `plugin:${p.id}:${c.id}`,
          label: c.label,
          hint: `Plugin · ${p.name}`,
          run: () => void this.invoke(p.id, c.id, c.context),
        });
      }
    }
    return out;
  }

  // The one thing a palette entry does: call the backend command. Declarative
  // — this never touches the manifest's `run` template (the Rust side expands
  // it). Builds the PlaceholderCtx from what the bridge actually exposes: the
  // open repo always, plus the selected commit's sha for a `commit` command.
  // (The bridge exposes no selected-file state, so a `file` command falls back
  // to repo-only — see selectedSha's counterpart absence.)
  async invoke(pluginId: string, commandId: string, context?: PluginContext): Promise<void> {
    const repo = bridge.CUR_REPO as unknown as string | null;
    if (!repo) {
      bridge.tama.warn("Open a repository first to run a plugin command.");
      return;
    }
    const ctx: PlaceholderCtx = { repo, sha: null, file: null, files: [], diff: null, branch: null, ref: null };
    if (context === "commit") {
      const sha = this.selectedSha();
      if (sha) ctx.sha = sha;
    }
    if (!IN_TAURI) {
      bridge.tama.say("This is where the plugin command would run (demo).");
      return;
    }
    try {
      const res = await commands.runPluginCommand(pluginId, commandId, ctx);
      if (res.status !== "ok") {
        bridge.tama.warn(String(res.error ?? "Plugin command failed."));
        return;
      }
      const out = res.data;
      const text = this.truncate((out.stdout || "").trim());
      if (out.success) {
        bridge.tama.say(text || "Plugin command finished.");
      } else {
        bridge.tama.warn(text || `Plugin command exited ${out.exitCode ?? "on a signal"}.`);
      }
    } catch (e) {
      bridge.tama.warn("Plugin command failed — " + e);
    }
  }

  // The currently-selected commit's sha, read from the same canvas state the
  // graph itself draws from (bridge.state.selectedRow -> bridge.BACKEND.rows).
  // Null when nothing (or the pinned "Uncommitted changes" row, -2) is
  // selected, or the row has no backing commit yet.
  private selectedSha(): string | null {
    try {
      const st = bridge.state as unknown as { selectedRow: number } | null;
      const row = st ? st.selectedRow : -1;
      if (row == null || row < 0) return null;
      const backend = bridge.BACKEND as unknown as { rows: Array<{ sha?: string }> } | null;
      const m = backend && backend.rows ? backend.rows[row] : null;
      return (m && m.sha) || null;
    } catch {
      return null;
    }
  }

  // Plugin stdout can be anything — collapse whitespace and cap it so Tama's
  // one-line nook stays a one-liner.
  private truncate(s: string, max = 160): string {
    const one = s.replace(/\s+/g, " ").trim();
    return one.length > max ? one.slice(0, max - 1) + "…" : one;
  }
}

export const pluginCommandsCtrl = new PluginCommandsState();
