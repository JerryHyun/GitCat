import { test, expect } from "@playwright/test";

// Scroll-blit smoke test (design mode — no Tauri, the synthetic demo graph).
// The canvas has no unit tests (it boots the whole app on import), so this is
// the automated guard for the offscreen-buffer + blit render path: it verifies
// the composite produces a real (multi-colour) canvas, scrolling changes the
// view without console errors, and the blit fast-path actually engages
// (HUD "blit N%" > 0 after a scroll).

// Distinct-colour count + a rolling hash over a sparse pixel sample. "Content
// present" = several distinct colours (dots/edges/text over the background), a
// far more robust signal than "differs from the top-left pixel".
async function canvasStats(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const c = document.getElementById("cv") as HTMLCanvasElement;
    const g = c.getContext("2d")!;
    const d = g.getImageData(0, 0, c.width, c.height).data;
    const seen = new Set<string>();
    let h = 0;
    for (let i = 0; i < d.length; i += 397 * 4) {
      h = (h * 31 + d[i] + d[i + 1] * 7 + d[i + 2] * 13) >>> 0;
      seen.add(d[i] + "," + d[i + 1] + "," + d[i + 2]);
    }
    return { h, distinct: seen.size };
  });
}

test("graph canvas composites, scrolls, and engages the blit fast-path", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/");
  await expect(page.locator("#cv")).toBeVisible();
  await page.waitForTimeout(700); // let the demo graph render a full frame

  const before = await canvasStats(page);
  expect(before.distinct, "canvas should render real content (many colours), not a blank fill").toBeGreaterThan(5);

  // Scroll down by dispatching wheel events directly on the canvas (an overlay
  // intercepts a synthesized page.mouse.wheel), then let the eased lerp run so
  // several rAF frames take the pure-vertical-scroll blit path.
  await page.evaluate(() => {
    const cv = document.getElementById("cv")!;
    for (let i = 0; i < 12; i++) cv.dispatchEvent(new WheelEvent("wheel", { deltaY: 600, bubbles: true, cancelable: true }));
  });
  await page.waitForTimeout(500);

  const after = await canvasStats(page);
  expect(after.distinct, "canvas should still be painted after scrolling").toBeGreaterThan(5);
  expect(after.h, "scrolling should change what's on screen").not.toBe(before.h);

  // The HUD reports the blit-hit ratio; after a real scroll it must be > 0 — i.e.
  // the fast-path engaged rather than silently falling back to full re-renders.
  const hud = (await page.locator("#hud").textContent()) || "";
  expect(hud, "HUD should report a blit ratio").toMatch(/blit\s+\d+%/);
  expect(Number(/blit\s+(\d+)%/.exec(hud)?.[1] ?? "0"), "blit fast-path should engage during scroll").toBeGreaterThan(0);

  expect(errors, "no console/page errors during scroll").toEqual([]);
});
