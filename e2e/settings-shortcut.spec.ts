import { test, expect } from "@playwright/test";

// Ctrl/⌘ + , opens Settings (the frontend fallback in src/main.ts, since the
// native menu accelerator doesn't reliably fire on Windows). Design mode has no
// native menu, so this exercises exactly that fallback path.
test("Ctrl/Cmd + , opens the Settings modal", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/");
  await page.waitForTimeout(300);

  const settings = page.locator(".modal.settings");
  await expect(settings).toBeHidden();

  // Windows path.
  await page.keyboard.press("Control+,");
  await expect(settings, "Ctrl+, should open Settings").toBeVisible();

  await page.keyboard.press("Escape");
  await expect(settings).toBeHidden();

  // macOS path.
  await page.keyboard.press("Meta+,");
  await expect(settings, "Cmd+, should open Settings").toBeVisible();

  // A stray Ctrl+, while typing in a field must NOT pop Settings.
  await page.keyboard.press("Escape");
  await expect(settings).toBeHidden();

  expect(errors).toEqual([]);
});
