#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright";
import { manifestDigest, presentations, trialOrder } from "../../evals/listening/randomization.js";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PYTHON = process.env.PYTHON ?? "python3";
const BROWSER_NAME = process.env.LISTENING_BROWSER ?? "chromium";
const BROWSER = BROWSER_NAME === "webkit" ? webkit : chromium;
const port = 8174;
const server = spawn(PYTHON, ["-m", "http.server", String(port), "--bind", "127.0.0.1"], { cwd: ROOT, stdio: "ignore" });
const downloads = await mkdtemp(join(tmpdir(), "ij-listening-downloads-"));
const campaignRoot = await mkdtemp(join(ROOT, ".listening-e2e-"));
let browser;

function runPython(args) {
  const result = spawnSync(PYTHON, args, { cwd: ROOT, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`python failed (${args.join(" ")}):\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

async function completeSetup(page, listener) {
  await page.fill("#listener", listener);
  await page.fill("#experience", "harness validation");
  await page.fill("#environment", "headless browser");
  await page.fill("#device", `Playwright ${BROWSER_NAME}`);
  await page.check("#volume");
  await page.click("#start");
}

async function exportSession(page, destination) {
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export raw session JSON" }).click();
  const download = await downloadPromise;
  await download.saveAs(destination);
  return JSON.parse(await readFile(destination, "utf8"));
}

async function playEveryVisibleSampleToCompletion(page) {
  const samples = page.locator(".sample");
  for (let index = 0; index < await samples.count(); index++) {
    const sample = samples.nth(index);
    await sample.locator("button.play").click();
    await page.waitForFunction(
      (position) => document.querySelectorAll(".sample .plays")[position]?.textContent?.startsWith("1 complete"),
      index,
      { timeout: 5000 },
    );
  }
}

function resampleAudio(source, destination, sampleRate) {
  const code = "import sys; sys.path.insert(0, 'scripts/dev'); import loop_metrics; import soundfile as sf; x, sr = loop_metrics.load_mono(sys.argv[1]); y = loop_metrics.resample_to(x, sr, int(sys.argv[3])); sf.write(sys.argv[2], y, int(sys.argv[3]), subtype='PCM_16')";
  runPython(["-c", code, source, destination, String(sampleRate)]);
}

function iteration(commit, cases, sampleRate) {
  return {
    family: "piano",
    metric_version: "browser-round-trip",
    manifest: { sha256: "a".repeat(64) },
    source: { commit },
    cases: cases.map((id, index) => ({
      id,
      role: index === 0 ? "tune" : "held_out",
      reference_sha256: `${index + 1}`.repeat(64),
      render_metadata: {
        family: "piano", midi: 60 + index, vel: 90, onsetSeconds: 0.03,
        noteOffSeconds: 1.03, seconds: 2, sampleRate, float32: true,
      },
    })),
  };
}

async function createCampaignBundle() {
  const baseline = join(campaignRoot, "iteration-old");
  const candidate = join(campaignRoot, "iteration-new");
  const cases = ["case-a", "case-b"];
  const sampleRate = 44100;
  await mkdir(join(baseline, "renders"), { recursive: true });
  await mkdir(join(candidate, "renders"), { recursive: true });
  for (const id of cases) {
    resampleAudio(join(ROOT, "evals/listening/pilot/audio/reference.wav"), join(baseline, `renders/${id}.wav`), sampleRate);
    resampleAudio(join(ROOT, "evals/listening/pilot/audio/condition-02.wav"), join(candidate, `renders/${id}.wav`), sampleRate);
  }
  await writeFile(join(baseline, "iteration.json"), `${JSON.stringify(iteration("1".repeat(40), cases, sampleRate), null, 2)}\n`);
  await writeFile(join(candidate, "iteration.json"), `${JSON.stringify(iteration("2".repeat(40), cases, sampleRate), null, 2)}\n`);
  runPython(["scripts/dev/listening.py", "prepare-campaign", candidate, "--baseline", baseline, "--out", join(candidate, "listening")]);
  return { candidate, bundle: join(candidate, "listening"), analysis: join(candidate, "listening-analysis.json") };
}

async function waitForServer(url) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`listening test server did not become ready: ${url}`);
}

async function answerAbTrial(page) {
  await playEveryVisibleSampleToCompletion(page);
  await page.getByRole("button", { name: "Sample 1", exact: true }).click();
}

try {
  await waitForServer(`http://127.0.0.1:${port}/evals/listening/`);
  browser = await BROWSER.launch();
  const context = await browser.newContext({ acceptDownloads: true });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => message.type() === "error" && errors.push(message.text()));

  const seed = 0x10010001;
  await page.goto(`http://127.0.0.1:${port}/evals/listening/?experiment=pilot/experiment.json&seed=${seed}`, { waitUntil: "networkidle" });
  await completeSetup(page, "browser-pilot");
  const visible = await page.locator("body").innerText();
  if (visible.includes("hidden_reference") || visible.includes("anchor") || visible.includes("candidate")) throw new Error("pilot visual identity leak");
  await playEveryVisibleSampleToCompletion(page);
  for (const slider of await page.locator('input[type="range"]').all()) await slider.fill("95");
  await page.getByRole("button", { name: "Submit session" }).click();
  const pilotSession = await exportSession(page, join(downloads, "pilot-session.json"));
  const pilotExperiment = JSON.parse(await readFile(join(ROOT, "evals/listening/pilot/experiment.json"), "utf8"));
  const expected = presentations(pilotExperiment, seed)["synthetic-tone-mushra"];
  const pilotStored = await page.evaluate((key) => localStorage.getItem(key), `ij-listening:${pilotSession.session_id}`);
  const pilotAssertions = {
    digestMatches: pilotSession.experiment_digest === await manifestDigest(pilotExperiment),
    presentationMatches: JSON.stringify(pilotSession.trials[0].presentation) === JSON.stringify(expected),
    trialOrderMatches: JSON.stringify(pilotSession.trial_order) === JSON.stringify(trialOrder(pilotExperiment, seed)),
    completePlaybackStored: Object.values(pilotSession.trials[0].playback).every((item) => item.completed === 1 && item.listened_ms >= 790),
    rawRatingsStored: Object.keys(pilotSession.trials[0].response.ratings).length === 3,
    setupStored: pilotSession.setup.device === `Playwright ${BROWSER_NAME}` && pilotSession.setup.volume_check === true,
    localRoundTrip: pilotStored === JSON.stringify(pilotSession),
    noVerdict: !("quality_verdict" in pilotSession),
  };

  const { bundle, analysis } = await createCampaignBundle();
  const bundleUrl = relative(ROOT, join(bundle, "index.html")).split(sep).join("/");
  const campaignUrl = `http://127.0.0.1:${port}/${bundleUrl}?seed=305419896`;
  await page.goto(campaignUrl, { waitUntil: "networkidle" });
  const publicExperiment = JSON.parse(await readFile(join(bundle, "experiment.json"), "utf8"));
  const publicText = JSON.stringify(publicExperiment);
  const publicPaths = publicExperiment.trials.flatMap((trial) => trial.stimuli.map((item) => item.path));
  if (/candidate|incumbent|role|source_sha|222222/i.test(publicText) || publicPaths.some((path) => /candidate|incumbent/i.test(path))) throw new Error("campaign participant manifest leaks condition identity");
  await completeSetup(page, "campaign-browser-pilot");
  const campaignVisible = await page.locator("body").innerText();
  const mediaUrls = await page.locator("audio").evaluateAll((elements) => elements.map((element) => element.src));
  if (/candidate|incumbent/i.test(campaignVisible) || mediaUrls.some((url) => /candidate|incumbent/i.test(url))) {
    throw new Error(`campaign browser surface leaks condition identity: ${campaignVisible} ${JSON.stringify(mediaUrls)}`);
  }
  await answerAbTrial(page);
  await page.getByRole("button", { name: "Save and continue" }).click();
  const firstProgress = await page.locator(".progress").innerText();
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: /Resume saved session/ }).click();
  const resumedProgress = await page.locator(".progress").innerText();
  await answerAbTrial(page);
  await page.getByRole("button", { name: "Submit session" }).click();
  const campaignSessionPath = join(downloads, "campaign-session.json");
  const campaignSession = await exportSession(page, campaignSessionPath);
  const analysisPath = join(downloads, "campaign-analysis.json");
  runPython(["scripts/dev/listening.py", "analyze", analysis, campaignSessionPath, "--out", analysisPath]);
  const campaignAnalysis = JSON.parse(await readFile(analysisPath, "utf8"));
  const campaignAssertions = {
    sampleRate44100: publicExperiment.sample_rate === 44100,
    participantManifestIsOpaque: !/candidate|incumbent|role|source_sha|222222/i.test(publicText),
    digestMatchesAcrossLanguages: campaignSession.experiment_digest === await manifestDigest(publicExperiment)
      && campaignSession.experiment_digest === campaignAnalysis.experiment_digest,
    resumeRecoveredCompletedTrial: firstProgress.includes("Trial 2 of 2") && resumedProgress.includes("Trial 2 of 2") && campaignSession.trials.length === 2,
    completePlaybackStored: campaignSession.trials.every((trial) => Object.values(trial.playback).every((item) => item.completed === 1 && item.listened_ms >= 790)),
    pythonAcceptedBrowserSession: campaignAnalysis.n_submitted === 1 && campaignAnalysis.n_included === 1,
    rawSessionRetained: campaignAnalysis.raw_sessions[0].session_id === campaignSession.session_id,
    manualFallbackPopulated: JSON.parse(await page.locator("#manual-export").inputValue()).session_id === campaignSession.session_id,
    noVerdict: campaignAnalysis.quality_verdict === null,
  };

  const fallbackContext = await browser.newContext();
  await fallbackContext.addInitScript(() => {
    Storage.prototype.setItem = function setItem() { throw new DOMException("blocked", "QuotaExceededError"); };
  });
  const fallbackPage = await fallbackContext.newPage();
  await fallbackPage.goto(campaignUrl, { waitUntil: "networkidle" });
  await completeSetup(fallbackPage, "storage-fallback-pilot");
  await answerAbTrial(fallbackPage);
  await fallbackPage.getByRole("button", { name: "Save and continue" }).click();
  const interruptedJsonText = await fallbackPage.locator("#manual-export").inputValue();
  const interruptedJson = JSON.parse(interruptedJsonText);
  const exportVisibleDuringProgress = await fallbackPage.locator("#manual-export-section").isVisible();
  await fallbackPage.reload({ waitUntil: "networkidle" });
  await fallbackPage.getByText("Recover from a manual session copy").click();
  await fallbackPage.locator("#restore-json").fill(interruptedJsonText);
  await fallbackPage.getByRole("button", { name: "Restore session JSON" }).click();
  const restoredProgress = await fallbackPage.locator(".progress").innerText();
  await answerAbTrial(fallbackPage);
  await fallbackPage.getByRole("button", { name: "Submit session" }).click();
  const fallbackJson = JSON.parse(await fallbackPage.locator("#manual-export").inputValue());
  const fallbackStatus = await fallbackPage.locator("#status").innerText();
  const fallbackAssertions = {
    inProgressCopyAvailable: exportVisibleDuringProgress && interruptedJson.trials.length === 1 && !interruptedJson.submitted_at,
    restoredAfterInterruption: restoredProgress.includes("Trial 2 of 2") && fallbackJson.session_id === interruptedJson.session_id,
    completedWithoutStorage: fallbackJson.trials.length === 2 && Boolean(fallbackJson.submitted_at),
    manualCopyAvailable: fallbackJson.listener.id === "storage-fallback-pilot",
    explicitInMemoryWarning: fallbackStatus.includes("in memory") && fallbackStatus.includes("storage is unavailable"),
  };
  await fallbackContext.close();

  const assertions = {
    noConsoleErrors: errors.length === 0,
    pilot: pilotAssertions,
    campaign: campaignAssertions,
    storageFallback: fallbackAssertions,
  };
  const verdict = errors.length === 0
    && Object.values(pilotAssertions).every(Boolean)
    && Object.values(campaignAssertions).every(Boolean)
    && Object.values(fallbackAssertions).every(Boolean) ? "PASS" : "FAIL";
  console.log(JSON.stringify({ verdict, browser: BROWSER_NAME, seed, pilotPresentation: expected, assertions, errors }, null, 2));
  if (verdict !== "PASS") process.exitCode = 1;
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
  await rm(downloads, { recursive: true, force: true });
  await rm(campaignRoot, { recursive: true, force: true });
}
