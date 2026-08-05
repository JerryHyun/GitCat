import { test, expect } from "./fixtures/tauriMock";

test("opening a repo populates the sidebar from the real fixture repo's refs", async ({ page, repo }) => {
  repo.writeFile("README.md", "# fixture\n");
  repo.commit("Initial commit");
  repo.writeFile("src/lib.ts", "export const answer = 42;\n");
  repo.commit("Add lib.ts");
  repo.branch("feature/widget");

  await page.goto("/");
  // The topbar chip opens the repositories dashboard rather than a folder
  // picker — every way into a repo funnels through it now (see legacy/main.ts's
  // own `.repo-pick` handler and dashboard.svelte.ts's addRepository), and its
  // "+ Add repository…" is what actually picks a folder and then opens it.
  await page.locator(".repo-pick").click();
  await page.locator(".db-add").click();

  const repoName = repo.dir.replace(/[/\\]+$/, "").split(/[/\\]/).pop()!;
  // .repo-name specifically, not a bare "span" — openRepo()'s loading
  // spinner is ALSO briefly a "span" inside .repo-pick while this resolves
  // (see legacy/main.ts's own comment on that exact line), so a
  // less-specific selector here could pass by matching the spinner's own
  // transient text instead of actually asserting the real chip updated.
  await expect(page.locator(".repo-pick .repo-name")).toHaveText(repoName);

  // Every sidebar section is a <details> with no `open` attribute, so its rows
  // render into the DOM but stay HIDDEN until the group is expanded. Asserting
  // visibility without this click is how the rest of this spec quietly went
  // stale: the rows were there, so a debug dump looked right, while every
  // toBeVisible on them could only ever fail.
  await page.locator(".ref-group", { has: page.locator("#refLocal") }).locator("summary").click();

  // The count chip is the folder-shape-independent half of "list_refs was read":
  // it reports how many local branches came back, whatever the tree does with
  // them, so it holds for a flat list and for a grouped one alike.
  await expect(page.locator("#cntLocal")).toHaveText("2");
  // `main` is never inside a folder, so it is a top-level row in either shape.
  await expect(page.locator('#refLocal [data-branch="main"]')).toBeVisible();
});
