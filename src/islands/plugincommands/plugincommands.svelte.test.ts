// Tests for the plugin-commands (PER-42) controller.
//
// Same isolation strategy as reflog.svelte.test.ts: legacy/bridge is mocked so
// legacy/main.ts (the whole vanilla canvas app that boots on import) never
// evaluates, and the backend commands (list_plugins / run_plugin_command) are
// mocked so nothing touches a real Tauri IPC.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../legacy/bridge", () => ({
  CUR_REPO: "/repo",
  tama: { set: vi.fn(), say: vi.fn(), warn: vi.fn(), event: vi.fn() },
  // Canvas selection state the controller reads for a `commit` context —
  // mutable plain objects so each test can set the selected row/rows.
  state: { selectedRow: -1 },
  BACKEND: { rows: [] as Array<{ sha: string }> },
}));

vi.mock("../../ipc/bindings", () => ({
  commands: {
    listPlugins: vi.fn(),
    runPluginCommand: vi.fn(),
  },
}));

// IN_TAURI is a live `const` computed from `window.__TAURI__` at import time —
// mock it so the real (non-demo) branches run.
vi.mock("../../ipc/env", () => ({ IN_TAURI: true }));

import { commands } from "../../ipc/bindings";
import * as bridge from "../../legacy/bridge";
import type { CommandOutput, Plugin } from "../../ipc/bindings";
import { pluginCommandsCtrl } from "./plugincommands.svelte.ts";

function ok<T>(data: T): { status: "ok"; data: T } {
  return { status: "ok", data };
}
function err(error: string): { status: "error"; error: string } {
  return { status: "error", error };
}

function plugin(partial: Partial<Plugin> & Pick<Plugin, "id" | "name">): Plugin {
  return { version: "1.0.0", description: null, enabled: true, commands: [], hooks: [], ...partial };
}

function output(partial: Partial<CommandOutput>): CommandOutput {
  return { stdout: "", exitCode: 0, success: true, ...partial };
}

function resetCtrl() {
  pluginCommandsCtrl.actions = [];
  pluginCommandsCtrl.loaded = false;
  pluginCommandsCtrl.onActionsChanged = null;
  (bridge.state as unknown as { selectedRow: number }).selectedRow = -1;
  (bridge.BACKEND as unknown as { rows: Array<{ sha: string }> }).rows = [];
}

beforeEach(() => {
  vi.clearAllMocks();
  resetCtrl();
});

describe("isolation", () => {
  it("never touches the DOM #cv canvas that legacy/main.ts would require", () => {
    expect(document.getElementById("cv")).toBeNull();
    expect(pluginCommandsCtrl).toBeDefined();
  });
});

describe("ensureLoaded — building the palette actions", () => {
  it("keeps ENABLED plugins and only palette/both-placement commands", async () => {
    vi.mocked(commands.listPlugins).mockResolvedValueOnce(
      ok([
        plugin({
          id: "acme",
          name: "Acme Tools",
          commands: [
            { id: "greet", label: "Greet", run: "echo hi", context: "none", placement: "palette" },
            { id: "menu-only", label: "Menu Only", run: "echo x", context: "none", placement: "menu" },
            { id: "both", label: "Both Places", run: "echo y", context: "commit", placement: "both" },
            // placement omitted -> defaults to "palette", so it's kept.
            { id: "defaulted", label: "Defaulted", run: "echo z", context: "none" },
          ],
        }),
        // enabled: false -> the whole plugin's commands are dropped.
        plugin({
          id: "off",
          name: "Disabled Plugin",
          enabled: false,
          commands: [{ id: "nope", label: "Nope", run: "echo no", placement: "palette" }],
        }),
        // enabled omitted -> treated as enabled.
        plugin({
          id: "impl",
          name: "Implicitly On",
          enabled: undefined,
          commands: [{ id: "run", label: "Run It", run: "echo run", placement: "palette" }],
        }),
      ]),
    );

    await pluginCommandsCtrl.ensureLoaded();

    const ids = pluginCommandsCtrl.actions.map((a) => a.id);
    expect(ids).toEqual(["plugin:acme:greet", "plugin:acme:both", "plugin:acme:defaulted", "plugin:impl:run"]);
    expect(ids).not.toContain("plugin:acme:menu-only");
    expect(ids).not.toContain("plugin:off:nope");
  });

  it("maps each command to the ActionItem id/label/hint shape", async () => {
    vi.mocked(commands.listPlugins).mockResolvedValueOnce(
      ok([plugin({ id: "acme", name: "Acme Tools", commands: [{ id: "greet", label: "Greet", run: "echo hi", placement: "palette" }] })]),
    );

    await pluginCommandsCtrl.ensureLoaded();

    expect(pluginCommandsCtrl.actions[0]).toMatchObject({
      type: "action",
      id: "plugin:acme:greet",
      label: "Greet",
      hint: "Plugin · Acme Tools",
    });
    expect(typeof pluginCommandsCtrl.actions[0].run).toBe("function");
  });

  it("is cached — a second ensureLoaded does not re-hit the backend", async () => {
    vi.mocked(commands.listPlugins).mockResolvedValueOnce(ok([]));

    await pluginCommandsCtrl.ensureLoaded();
    await pluginCommandsCtrl.ensureLoaded();

    expect(commands.listPlugins).toHaveBeenCalledTimes(1);
  });

  it("a backend error leaves the palette actions empty without throwing", async () => {
    vi.mocked(commands.listPlugins).mockResolvedValueOnce(err("registry unreadable"));

    await pluginCommandsCtrl.ensureLoaded();

    expect(pluginCommandsCtrl.actions).toEqual([]);
  });
});

describe("reload — force path notifies the palette", () => {
  it("rebuilds actions and fires onActionsChanged", async () => {
    const spy = vi.fn();
    pluginCommandsCtrl.onActionsChanged = spy;
    vi.mocked(commands.listPlugins).mockResolvedValueOnce(
      ok([plugin({ id: "acme", name: "Acme", commands: [{ id: "greet", label: "Greet", run: "echo", placement: "palette" }] })]),
    );

    await pluginCommandsCtrl.reload();

    expect(pluginCommandsCtrl.actions.map((a) => a.id)).toEqual(["plugin:acme:greet"]);
    expect(spy).toHaveBeenCalledTimes(1);
  });
});

describe("invoke — declarative backend call", () => {
  it("sends the open repo and calls runPluginCommand", async () => {
    vi.mocked(commands.runPluginCommand).mockResolvedValueOnce(ok(output({ stdout: "done", success: true })));

    await pluginCommandsCtrl.invoke("acme", "greet", "none");

    expect(commands.runPluginCommand).toHaveBeenCalledWith(
      "acme",
      "greet",
      expect.objectContaining({ repo: "/repo", sha: null }),
    );
    expect(bridge.tama.say).toHaveBeenCalled();
    expect(bridge.tama.warn).not.toHaveBeenCalled();
  });

  it("for a `commit` context, gathers the selected commit's sha from the bridge", async () => {
    (bridge.state as unknown as { selectedRow: number }).selectedRow = 1;
    (bridge.BACKEND as unknown as { rows: Array<{ sha: string }> }).rows = [{ sha: "aaaa111" }, { sha: "deadbeef" }];
    vi.mocked(commands.runPluginCommand).mockResolvedValueOnce(ok(output({ stdout: "ok", success: true })));

    await pluginCommandsCtrl.invoke("acme", "onCommit", "commit");

    expect(commands.runPluginCommand).toHaveBeenCalledWith(
      "acme",
      "onCommit",
      expect.objectContaining({ repo: "/repo", sha: "deadbeef" }),
    );
  });

  it("a `commit` context with nothing selected still sends repo (sha null)", async () => {
    (bridge.state as unknown as { selectedRow: number }).selectedRow = -1;
    vi.mocked(commands.runPluginCommand).mockResolvedValueOnce(ok(output({ stdout: "ok", success: true })));

    await pluginCommandsCtrl.invoke("acme", "onCommit", "commit");

    expect(commands.runPluginCommand).toHaveBeenCalledWith(
      "acme",
      "onCommit",
      expect.objectContaining({ repo: "/repo", sha: null }),
    );
  });

  it("a non-zero exit (success:false) warns via Tama", async () => {
    vi.mocked(commands.runPluginCommand).mockResolvedValueOnce(ok(output({ stdout: "boom", exitCode: 2, success: false })));

    await pluginCommandsCtrl.invoke("acme", "greet", "none");

    expect(bridge.tama.warn).toHaveBeenCalled();
    expect(bridge.tama.say).not.toHaveBeenCalled();
  });

  it("an IPC error warns via Tama", async () => {
    vi.mocked(commands.runPluginCommand).mockResolvedValueOnce(err("no such command"));

    await pluginCommandsCtrl.invoke("acme", "greet", "none");

    expect(bridge.tama.warn).toHaveBeenCalled();
  });

  it("a thrown IPC rejection warns via Tama without escaping", async () => {
    vi.mocked(commands.runPluginCommand).mockRejectedValueOnce(new Error("boom"));

    await pluginCommandsCtrl.invoke("acme", "greet", "none");

    expect(bridge.tama.warn).toHaveBeenCalled();
  });
});

describe("invoke — missing repo", () => {
  it("warns and does NOT call runPluginCommand when no repo is open", async () => {
    vi.resetModules();
    vi.doMock("../../legacy/bridge", () => ({
      CUR_REPO: null,
      tama: { set: vi.fn(), say: vi.fn(), warn: vi.fn(), event: vi.fn() },
      state: { selectedRow: -1 },
      BACKEND: { rows: [] },
    }));
    const bridgeNull = await import("../../legacy/bridge");
    const bindingsNull = await import("../../ipc/bindings");
    const { pluginCommandsCtrl: ctrl } = await import("./plugincommands.svelte.ts");

    await ctrl.invoke("acme", "greet", "none");

    expect(bridgeNull.tama.warn).toHaveBeenCalled();
    expect(bindingsNull.commands.runPluginCommand).not.toHaveBeenCalled();
  });
});
