import { describe, it, expect } from "vitest";
import { tamaConfirmCtrl } from "./tamaconfirm.svelte.ts";

describe("tamaConfirmCtrl", () => {
  it("ask() opens with the given copy and stays pending until answered", async () => {
    const p = tamaConfirmCtrl.ask({ title: "T", message: "M", confirmLabel: "Yes", kind: "warning" });
    expect(tamaConfirmCtrl.open).toBe(true);
    expect(tamaConfirmCtrl.title).toBe("T");
    expect(tamaConfirmCtrl.message).toBe("M");
    expect(tamaConfirmCtrl.confirmLabel).toBe("Yes");
    expect(tamaConfirmCtrl.kind).toBe("warning");

    tamaConfirmCtrl.confirm();
    expect(await p).toBe(true);
    expect(tamaConfirmCtrl.open).toBe(false);
  });

  it("cancel() resolves false and closes", async () => {
    const p = tamaConfirmCtrl.ask({ title: "T", message: "M" });
    tamaConfirmCtrl.cancel();
    expect(await p).toBe(false);
    expect(tamaConfirmCtrl.open).toBe(false);
  });

  it("defaults the labels and kind when omitted", async () => {
    const p = tamaConfirmCtrl.ask({ title: "T", message: "M" });
    expect(tamaConfirmCtrl.confirmLabel).toBe("Confirm");
    expect(tamaConfirmCtrl.cancelLabel).toBe("Cancel");
    expect(tamaConfirmCtrl.kind).toBe("info");
    tamaConfirmCtrl.cancel();
    await p;
  });

  it("a second ask() settles the first as false (never leaves it hanging)", async () => {
    const first = tamaConfirmCtrl.ask({ title: "A", message: "A" });
    const second = tamaConfirmCtrl.ask({ title: "B", message: "B" });
    expect(await first).toBe(false); // superseded
    expect(tamaConfirmCtrl.title).toBe("B");

    tamaConfirmCtrl.confirm();
    expect(await second).toBe(true);
  });
});
