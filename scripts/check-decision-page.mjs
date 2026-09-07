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
    ["d-063", "Flow Agent - First Release"],
    ["d-065", "Watershed - Shared Release Safeguards"],
    ["d-046", "Flow Agent - Permissions, Integrations And Offline Use"],
    ["d-020", "Flow Agent - Permissions, Integrations And Offline Use"],
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

  const url = page.url().split("#")[0];
  await page.goto(`${url}#d-063`, { waitUntil: "load" });
  const mac = page.locator("#d-063");
  assert.equal(await mac.locator("p").first().isVisible(), true, "direct links reveal their question");
  assert.equal(await page.locator("#d-065 p").first().isVisible(), false, "questions stay independent");
  const inferenceLink = mac.getByRole("link", { name: "D-061", exact: true });
  await inferenceLink.click();
  const inference = page.locator("#d-061");
  assert.equal(await inference.locator("p").first().isVisible(), true, "in-page links reveal their question");
  await inference.locator("summary").press("Space");
  assert.equal(await inference.locator("p").first().isVisible(), false);
  await inferenceLink.click();
  assert.equal(await inference.locator("p").first().isVisible(), true, "repeated links reopen a collapsed question");

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
