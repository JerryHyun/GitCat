// Dev-only structured logging of the app's key lifecycle events — graph loads,
// the incremental-refresh fast/full reload decisions, and what triggered a
// refresh (file watcher vs status poll vs an in-app mutation). Meant for
// answering "when/why did loading or a reload fire?" during development.
//
// Enabled in dev builds (`import.meta.env.DEV`) or, in any build, by setting
// `localStorage["gitcat.debug"] = "1"` (then reload). Disabled — the default in
// a shipped build — dlog() is a cheap no-op.
//
// In a `pnpm tauri dev` build the lines go through tauri-plugin-log (registered
// in src-tauri/src/lib.rs's debug-assertions block) to BOTH the dev terminal
// (stdout) and the app's log file, and attachConsole() mirrors them into the
// webview DevTools console. The log file lives in the OS app-log dir, e.g.
//   macOS:   ~/Library/Logs/com.jiucheng.gitcat/gitcat.log
//   Linux:   ~/.local/share/com.jiucheng.gitcat/logs/gitcat.log  (or $XDG_DATA_HOME)
//   Windows: %APPDATA%\com.jiucheng.gitcat\logs\gitcat.log
// so you can `tail -f` it. In plain-browser design mode (`pnpm run dev`, no
// Tauri) there's no backend, so lines just go to the browser console.
//
// Note: tauri-plugin-log is NOT registered when GITCAT_DEVTOOLS or
// GITCAT_TOKIO_CONSOLE is set (those inspectors own the global logger — see
// lib.rs); dlog then falls back to the DevTools console only.

import { IN_TAURI } from "./ipc/env";

const ENABLED =
  import.meta.env.DEV ||
  (typeof localStorage !== "undefined" && localStorage.getItem("gitcat.debug") === "1");

// Resolved lazily so the plugin bundle (and its IPC) is only pulled in when
// logging is actually on and we're inside Tauri.
let pluginInfo: ((message: string) => Promise<void>) | null = null;

async function init(): Promise<void> {
  if (!IN_TAURI) return;
  try {
    const mod = await import("@tauri-apps/plugin-log");
    pluginInfo = mod.info;
    // Pipe the Rust log stream (which now includes our own forwarded lines, plus
    // any `log::` calls from the Rust side) into the DevTools console too.
    await mod.attachConsole();
  } catch {
    // Plugin not registered (an inspector env var is set) — stay console-only.
  }
}
if (ENABLED) void init();

export function dlog(category: string, ...args: unknown[]): void {
  if (!ENABLED) return;
  const msg = `[${category}] ` + args.map((a) => (typeof a === "string" ? a : JSON.stringify(a))).join(" ");
  if (pluginInfo) {
    // → dev terminal + app log file; attachConsole() echoes it to DevTools.
    void pluginInfo(msg).catch(() => {});
  } else {
    // Browser design-mode, or before the plugin has finished loading.
    // eslint-disable-next-line no-console
    console.debug("%cgitcat", "color:#F5B843;font-weight:bold", msg);
  }
}
