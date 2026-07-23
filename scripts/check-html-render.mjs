import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const viewports = [
  { name: "desktop", width: 1440, height: 900 },
  { name: "mobile", width: 390, height: 844 },
];

function runPlaywrightCli(args, action) {
  const result =
    process.platform === "win32"
      ? spawnSync("cmd.exe", ["/d", "/s", "/c", ["pnpm", ...args].join(" ")], {
          cwd: repoRoot,
          stdio: "inherit",
        })
      : spawnSync("pnpm", args, {
          cwd: repoRoot,
          stdio: "inherit",
        });

  if (result.error) {
    throw new Error(`failed to start ${action}: ${result.error.message}`);
  }
  if (result.signal) {
    throw new Error(`${action} stopped by signal ${result.signal}`);
  }
  if (result.status !== 0) {
    throw new Error(`${action} failed with exit code ${result.status}`);
  }
}

function ensurePlaywrightChromium() {
  if (process.platform === "linux" && process.env.CI === "true") {
    runPlaywrightCli(
      ["exec", "playwright", "install-deps", "chromium"],
      "Playwright browser dependency install",
    );
  }

  const executablePath = chromium.executablePath();
  if (existsSync(executablePath)) {
    return;
  }

  runPlaywrightCli(["exec", "playwright", "install", "chromium"], "Playwright browser install");

  if (!existsSync(executablePath)) {
    throw new Error(`Playwright browser install did not create ${executablePath}`);
  }
}

function normalizeText(value) {
  return value.replace(/\s+/g, " ").trim();
}

async function expectedHeading(page, html, relativePath) {
  const text = await page.evaluate((source) => {
    const parsed = new DOMParser().parseFromString(source, "text/html");
    const heading = parsed.querySelector("h1");
    if (!heading) {
      return null;
    }

    const host = document.createElement("div");
    host.style.position = "fixed";
    host.style.left = "-10000px";
    host.append(document.importNode(heading, true));
    document.body.append(host);
    const rendered = host.querySelector("h1").innerText;
    host.remove();
    return rendered;
  }, html);
  if (text === null) {
    throw new Error(`${relativePath}: missing h1 for render assertion`);
  }
  return normalizeText(text);
}

async function assertHeadingExtraction(page) {
  for (const [html, expected] of [
    ["<h1>Plain heading</h1>", "Plain heading"],
    ["<h1>R&amp;D</h1>", "R&D"],
    ["<h1>Loop <span>Agent</span></h1>", "Loop Agent"],
    ["<h1>Loop<br>Agent</h1>", "Loop Agent"],
  ]) {
    const actual = await expectedHeading(page, html, "heading extraction assertion");
    if (actual !== expected) {
      throw new Error(`heading extraction assertion: expected ${expected}, received ${actual}`);
    }
  }
}

async function docsToCheck() {
  const docs = ["docs/decisions/open-decisions.html"];
  const conceptDir = path.join(repoRoot, "docs", "concept");
  const conceptFiles = await readdir(conceptDir);
  docs.push(
    ...conceptFiles
      .filter((name) => /^V-Spec_.*\.html$/.test(name))
      .sort()
      .map((name) => path.join("docs", "concept", name).replaceAll(path.sep, "/")),
  );

  return Promise.all(
    docs.map(async (relativePath) => {
      const absolutePath = path.join(repoRoot, relativePath);
      const html = await readFile(absolutePath, "utf8");
      return {
        absolutePath,
        html,
        relativePath,
      };
    }),
  );
}

async function assertVisibleLayout(page, expectedText, viewport, label) {
  const layout = await page.evaluate(({ expectedText, viewport }) => {
    const body = document.body;
    const heading = document.querySelector("h1");
    if (!body || !heading) {
      return null;
    }

    const visibleInViewport = (element) => {
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      const cssVisible =
        typeof element.checkVisibility === "function"
          ? element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
          : style.display !== "none" && style.visibility === "visible" && Number(style.opacity) > 0;
      return (
        cssVisible &&
        rect.width > 0 &&
        rect.height > 0 &&
        rect.right > 0 &&
        rect.bottom > 0 &&
        rect.left < viewport.width &&
        rect.top < viewport.height
      );
    };

    return {
      bodyVisible: visibleInViewport(body),
      headingText: heading.innerText.replace(/\s+/g, " ").trim(),
      headingVisible: visibleInViewport(heading),
      expectedText,
    };
  }, { expectedText, viewport });

  if (!layout) {
    throw new Error(`${label}: missing body or h1`);
  }
  if (!layout.bodyVisible || !layout.headingVisible) {
    throw new Error(`${label}: body or h1 is not visibly laid out in the viewport`);
  }
  if (layout.headingText !== layout.expectedText) {
    throw new Error(`${label}: rendered h1 does not match "${layout.expectedText}"`);
  }
}

async function checkDocument(browser, doc, viewport) {
  const label = `${doc.relativePath} ${viewport.name} ${viewport.width}x${viewport.height}`;
  const context = await browser.newContext({
    viewport: { width: viewport.width, height: viewport.height },
  });
  const page = await context.newPage();
  const consoleErrors = [];
  const pageErrors = [];

  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => pageErrors.push(error.message));

  try {
    await page.goto(pathToFileURL(doc.absolutePath).href, { waitUntil: "load" });

    const expectedText = await expectedHeading(page, doc.html, doc.relativePath);
    const renderedText = normalizeText(await page.locator("body").innerText());
    if (!renderedText.includes(expectedText)) {
      throw new Error(`${label}: missing expected text "${expectedText}"`);
    }

    await assertVisibleLayout(page, expectedText, viewport, label);

    if (consoleErrors.length > 0) {
      throw new Error(`${label}: console errors: ${consoleErrors.join("; ")}`);
    }
    if (pageErrors.length > 0) {
      throw new Error(`${label}: page errors: ${pageErrors.join("; ")}`);
    }
  } finally {
    await context.close();
  }
}

async function main() {
  ensurePlaywrightChromium();

  const docs = await docsToCheck();
  const browser = await chromium.launch();

  try {
    const assertionPage = await browser.newPage();
    await assertHeadingExtraction(assertionPage);
    await assertionPage.close();

    for (const doc of docs) {
      for (const viewport of viewports) {
        await checkDocument(browser, doc, viewport);
      }
    }
  } finally {
    await browser.close();
  }

  console.log(`HTML render check passed for ${docs.length} docs across ${viewports.length} viewports.`);
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
