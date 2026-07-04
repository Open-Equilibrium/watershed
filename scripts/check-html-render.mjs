import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { inflateSync } from "node:zlib";
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
  const executablePath = chromium.executablePath();
  if (existsSync(executablePath)) {
    return;
  }

  if (process.platform === "linux" && process.env.CI === "true") {
    runPlaywrightCli(
      ["exec", "playwright", "install-deps", "chromium"],
      "Playwright browser dependency install",
    );
  }

  runPlaywrightCli(["exec", "playwright", "install", "chromium"], "Playwright browser install");

  if (!existsSync(executablePath)) {
    throw new Error(`Playwright browser install did not create ${executablePath}`);
  }
}

function normalizeText(value) {
  return value.replace(/\s+/g, " ").trim();
}

function stripTags(value) {
  return value.replace(/<[^>]*>/g, "");
}

function expectedHeading(html, relativePath) {
  const match = html.match(/<h1\b[^>]*>([\s\S]*?)<\/h1>/i);
  if (!match) {
    throw new Error(`${relativePath}: missing h1 for render assertion`);
  }
  return normalizeText(stripTags(match[1]));
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
        relativePath,
        expectedText: expectedHeading(html, relativePath),
      };
    }),
  );
}

function parsePng(buffer) {
  const signature = "89504e470d0a1a0a";
  if (buffer.subarray(0, 8).toString("hex") !== signature) {
    throw new Error("screenshot is not a PNG");
  }

  let offset = 8;
  let width;
  let height;
  let bitDepth;
  let colorType;
  const idat = [];

  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.subarray(offset + 4, offset + 8).toString("ascii");
    const data = buffer.subarray(offset + 8, offset + 8 + length);

    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
    } else if (type === "IDAT") {
      idat.push(data);
    } else if (type === "IEND") {
      break;
    }

    offset += length + 12;
  }

  if (!width || !height || bitDepth !== 8) {
    throw new Error("unsupported PNG screenshot format");
  }

  const bytesPerPixelByColorType = new Map([
    [0, 1],
    [2, 3],
    [4, 2],
    [6, 4],
  ]);
  const bytesPerPixel = bytesPerPixelByColorType.get(colorType);
  if (!bytesPerPixel) {
    throw new Error(`unsupported PNG color type ${colorType}`);
  }

  return {
    bytesPerPixel,
    data: inflateSync(Buffer.concat(idat)),
    height,
    width,
  };
}

function unfilterPng({ bytesPerPixel, data, height, width }) {
  const stride = width * bytesPerPixel;
  const rows = Buffer.alloc(stride * height);
  let sourceOffset = 0;

  for (let y = 0; y < height; y += 1) {
    const filter = data[sourceOffset];
    sourceOffset += 1;
    const rowOffset = y * stride;
    const previousRowOffset = rowOffset - stride;

    for (let x = 0; x < stride; x += 1) {
      const raw = data[sourceOffset + x];
      const left = x >= bytesPerPixel ? rows[rowOffset + x - bytesPerPixel] : 0;
      const up = y > 0 ? rows[previousRowOffset + x] : 0;
      const upLeft = y > 0 && x >= bytesPerPixel ? rows[previousRowOffset + x - bytesPerPixel] : 0;

      if (filter === 0) {
        rows[rowOffset + x] = raw;
      } else if (filter === 1) {
        rows[rowOffset + x] = (raw + left) & 0xff;
      } else if (filter === 2) {
        rows[rowOffset + x] = (raw + up) & 0xff;
      } else if (filter === 3) {
        rows[rowOffset + x] = (raw + Math.floor((left + up) / 2)) & 0xff;
      } else if (filter === 4) {
        const predictor = paethPredictor(left, up, upLeft);
        rows[rowOffset + x] = (raw + predictor) & 0xff;
      } else {
        throw new Error(`unsupported PNG filter ${filter}`);
      }
    }

    sourceOffset += stride;
  }

  return rows;
}

function paethPredictor(left, up, upLeft) {
  const estimate = left + up - upLeft;
  const leftDistance = Math.abs(estimate - left);
  const upDistance = Math.abs(estimate - up);
  const upLeftDistance = Math.abs(estimate - upLeft);

  if (leftDistance <= upDistance && leftDistance <= upLeftDistance) {
    return left;
  }
  if (upDistance <= upLeftDistance) {
    return up;
  }
  return upLeft;
}

function assertScreenshotNotBlank(screenshot, label) {
  const png = parsePng(screenshot);
  const pixels = unfilterPng(png);
  const stride = png.width * png.bytesPerPixel;
  const firstPixel = pixels.subarray(0, png.bytesPerPixel);
  const sampleStep = Math.max(1, Math.floor((png.width * png.height) / 2000));

  for (let index = 1; index < png.width * png.height; index += sampleStep) {
    const offset = Math.floor(index / png.width) * stride + (index % png.width) * png.bytesPerPixel;
    for (let channel = 0; channel < Math.min(3, png.bytesPerPixel); channel += 1) {
      if (Math.abs(pixels[offset + channel] - firstPixel[channel]) > 2) {
        return;
      }
    }
  }

  throw new Error(`${label}: screenshot appears blank`);
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

    const renderedText = normalizeText(await page.locator("body").innerText());
    if (!renderedText.includes(doc.expectedText)) {
      throw new Error(`${label}: missing expected text "${doc.expectedText}"`);
    }

    const screenshot = await page.screenshot({ fullPage: false, type: "png" });
    assertScreenshotNotBlank(screenshot, label);

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
