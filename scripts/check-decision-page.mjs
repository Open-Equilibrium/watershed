import assert from "node:assert/strict";

export async function assertDecisionPage(page) {
  const decisions = page.locator(".decision");
  const count = await decisions.count();
  assert.ok(count > 0, "the decision index must contain questions");
  for (const decision of await decisions.all()) {
    const title = await decision.locator("h3").innerText();
    const summary = decision.locator("summary");
    assert.equal(await summary.count(), 1, `${title}: missing collapse control`);
    assert.equal(await decision.locator("p").first().isVisible(), false, `${title}: starts collapsed`);
    await summary.click();
    assert.equal(await decision.locator("p").first().isVisible(), true, `${title}: opens`);
    await summary.press("Enter");
    assert.equal(await decision.locator("p").first().isVisible(), false, `${title}: closes by keyboard`);
  }

  for (const [id, heading] of [
    ["d-066", "Flow Agent - First Release"],
    ["d-065", "Watershed - Shared Release Safeguards"],
    ["d-067", "Flow Agent - Permissions, Integrations And Offline Use"],
    ["d-020", "Flow Agent - Permissions, Integrations And Offline Use"],
    ["d-059", "Flow Agent - Permissions, Integrations And Offline Use"],
  ]) {
    const section = page.locator("section").filter({ has: page.locator(`#${id}`) });
    assert.equal(await section.getByRole("heading", { level: 2 }).innerText(), heading);
  }
  const anchors = await page.evaluate(() => {
    const ids = [...document.querySelectorAll("[id]")].map(element => element.id);
    return {
      unique: new Set(ids).size === ids.length,
      missing: [...document.querySelectorAll('a[href^="#"]')]
        .map(link => link.hash.slice(1)).filter(id => !document.getElementById(id)),
    };
  });
  assert.equal(anchors.unique, true, "decision anchors must be unique");
  assert.deepEqual(anchors.missing, [], "local navigation must resolve");
  assert.equal(await page.locator("#post-m1-2").count(), 1, "preserve the merged section's old anchor");
  assert.equal(await page.locator("#d-063").count(), 0, "the decided protection scope leaves the live index");

  const url = page.url().split("#")[0];
  await page.goto(`${url}#d-067`, { waitUntil: "load" });
  const marketplace = page.locator("#d-067");
  assert.equal(await marketplace.locator("p").first().isVisible(), true, "direct links reveal their question");
  assert.equal(await page.locator("#d-065 p").first().isVisible(), false, "questions stay independent");
  const toolsLink = marketplace.getByRole("link", { name: "D-066", exact: true });
  await toolsLink.click();
  assert.equal(new URL(page.url()).hash, "#d-066", "question links update the address");
  const standardTools = page.locator("#d-066");
  assert.equal(await standardTools.locator("p").first().isVisible(), true, "in-page links reveal their question");
  await standardTools.locator("summary").press("Space");
  assert.equal(await standardTools.locator("p").first().isVisible(), false);
  await toolsLink.click();
  assert.equal(await standardTools.locator("p").first().isVisible(), true, "repeated links reopen a collapsed question");
  await marketplace.locator("summary").press("Enter");
  assert.equal(await marketplace.locator("p").first().isVisible(), false);
  await page.goBack();
  assert.equal(new URL(page.url()).hash, "#d-067", "reopening the same question adds no history entry");
  await marketplace.locator("p").first().waitFor({ state: "visible" });
  await standardTools.locator("summary").press("Enter");
  assert.equal(await standardTools.locator("p").first().isVisible(), false);
  await page.goForward();
  assert.equal(new URL(page.url()).hash, "#d-066", "forward navigation restores the question address");
  await standardTools.locator("p").first().waitFor({ state: "visible" });

  await page.goto(url, { waitUntil: "load" });
  for (const decision of await page.locator(".decision").all()) {
    await decision.locator("summary").click();
  }
  assert.equal(await page.locator("details[open].decision").count(), count, "questions can be compared side by side");
  const layout = await page.evaluate(() => ({
    width: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  assert.ok(layout.scrollWidth <= layout.width + 1, "expanded questions must fit the viewport");
}
