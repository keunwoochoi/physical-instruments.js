#!/usr/bin/env node
/** Bundler-fixture check: load page, click #start, expect #status → "engine live".
 *  Usage: node scripts/dev/e2e-fixture.mjs <url> */
import { chromium } from "playwright";

const url = process.argv[2];
const browser = await chromium.launch({ args: ["--autoplay-policy=no-user-gesture-required"] });
const page = await browser.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e.message)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
await page.goto(url, { waitUntil: "load" });
await page.click("#start");
let status = "";
try {
  await page.waitForFunction(
    () => document.getElementById("status")?.textContent === "engine live",
    { timeout: 10000 },
  );
  status = "engine live";
} catch {
  status = (await page.locator("#status").textContent().catch(() => "?")) ?? "?";
}
await browser.close();
const verdict = status === "engine live" && errors.length === 0 ? "PASS" : "FAIL";
console.log(JSON.stringify({ url, status, errors, verdict }));
process.exit(verdict === "PASS" ? 0 : 1);
