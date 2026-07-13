#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { copyFile, cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright";
import { manifestDigest, presentations, trialOrder } from "../../evals/listening/randomization.js";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PYTHON = process.env.PYTHON ?? "python3";
const BROWSER_NAME = process.env.LISTENING_BROWSER ?? "chromium";
const BROWSER = BROWSER_NAME === "webkit" ? webkit : chromium;
const port = 8174;
const server = spawn(PYTHON, ["-m", "http.server", String(port), "--bind", "127.0.0.1"], { cwd: ROOT, stdio: "ignore" });
const servers = [server];
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
    manifest: { path: "case-manifest.json", sha256: "a".repeat(64), schema_path: "case-schema.json", schema_sha256: "b".repeat(64) },
    reference_registry: { path: "reference-registry.json", sha256: "c".repeat(64), schema_path: "reference-registry-schema.json", schema_sha256: "d".repeat(64) },
    source: { commit },
    cases: cases.map((id, index) => ({
      id,
      role: index === 0 ? "tune" : "held_out",
      reference_sha256: `${index + 1}`.repeat(64),
      reference_contract: {
        id: `ref.test.${id}`, corpus_id: "corpus.test", status: "verified", declared_path: `references/test/${id}.wav`,
        reference_sha256: `${index + 1}`.repeat(64), contract_sha256: `${index + 3}`.repeat(64),
        registry_sha256: "c".repeat(64), registry_schema_sha256: "d".repeat(64),
      },
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

async function createAbxBundle(candidate, sourceBundle, sourceAnalysis) {
  const bundle = join(candidate, "listening-abx");
  const analysisPath = join(candidate, "listening-abx-analysis.json");
  await cp(sourceBundle, bundle, { recursive: true });
  const experiment = JSON.parse(await readFile(join(bundle, "experiment.json"), "utf8"));
  const analysis = JSON.parse(await readFile(sourceAnalysis, "utf8"));
  for (let index = 0; index < experiment.trials.length; index++) {
    const trial = experiment.trials[index];
    const privateTrial = analysis.trials[index];
    const source = trial.stimuli[index % trial.stimuli.length];
    const xPath = `audio/x-${String(index + 1).padStart(3, "0")}.wav`;
    await copyFile(join(bundle, source.path), join(bundle, xPath));
    trial.protocol = "abx";
    trial.prompt = `Which sample is identical to X for case ${index + 1}?`;
    trial.x = { id: "x", path: xPath };
    privateTrial.x_source = source.id;
  }
  analysis.experiment = "listening-abx/experiment.json";
  analysis.experiment_digest = await manifestDigest(experiment);
  await writeFile(join(bundle, "experiment.json"), `${JSON.stringify(experiment, null, 2)}\n`);
  await writeFile(analysisPath, `${JSON.stringify(analysis, null, 2)}\n`);
  return { bundle, analysis: analysisPath, experiment };
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

  await page.goto(`http://127.0.0.1:${port}/evals/listening/?experiment=missing-experiment.json`, { waitUntil: "networkidle" });
  const missingManifestIsTerminal = await page.locator("#title").innerText() === "Listening experiment unavailable"
    && await page.locator("#setup").isHidden()
    && (await page.locator("#status").innerText()).includes("No response data");
  if (!missingManifestIsTerminal) throw new Error("manifest load failure was not actionable and terminal");
  errors.length = 0;

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

  const { candidate, bundle, analysis } = await createCampaignBundle();
  const hostilePrompt = '<img src=x onerror="window.__promptXss=true"> hostile prompt';
  const hostileExperiment = JSON.parse(await readFile(join(bundle, "experiment.json"), "utf8"));
  hostileExperiment.trials[0].prompt = hostilePrompt;
  await writeFile(join(bundle, "experiment.json"), `${JSON.stringify(hostileExperiment, null, 2)}\n`);
  const hostileAnalysis = JSON.parse(await readFile(analysis, "utf8"));
  hostileAnalysis.experiment_digest = await manifestDigest(hostileExperiment);
  await writeFile(analysis, `${JSON.stringify(hostileAnalysis, null, 2)}\n`);
  const campaignPort = port + 1;
  servers.push(spawn(PYTHON, ["-m", "http.server", String(campaignPort), "--bind", "127.0.0.1"], { cwd: bundle, stdio: "ignore" }));
  await waitForServer(`http://127.0.0.1:${campaignPort}/`);
  await writeFile(join(bundle, "malformed.json"), "{not-json");
  await writeFile(join(bundle, "invalid.json"), JSON.stringify({ title: "missing trials" }));
  for (const badManifest of ("malformed.json", "invalid.json")) {
    await page.goto(`http://127.0.0.1:${campaignPort}/?experiment=${badManifest}`, { waitUntil: "networkidle" });
    if (await page.locator("#title").innerText() !== "Listening experiment unavailable" || !await page.locator("#setup").isHidden()) {
      throw new Error(`${badManifest} did not render an actionable terminal error`);
    }
  }
  errors.length = 0;
  const campaignUrl = `http://127.0.0.1:${campaignPort}/?seed=305419896`;
  await page.goto(campaignUrl, { waitUntil: "networkidle" });
  const publicExperiment = JSON.parse(await readFile(join(bundle, "experiment.json"), "utf8"));
  const publicText = JSON.stringify(publicExperiment);
  const publicPaths = publicExperiment.trials.flatMap((trial) => trial.stimuli.map((item) => item.path));
  if (/candidate|incumbent|role|source_sha|222222/i.test(publicText) || publicPaths.some((path) => /candidate|incumbent/i.test(path))) throw new Error("campaign participant manifest leaks condition identity");
  await completeSetup(page, "campaign-browser-pilot");
  if (await page.evaluate(() => window.__promptXss === true) || await page.locator("#trial h2").innerText() !== hostilePrompt) {
    throw new Error("campaign prompt escaped text rendering");
  }
  const campaignVisible = await page.locator("body").innerText();
  const mediaUrls = await page.locator("audio").evaluateAll((elements) => elements.map((element) => element.src));
  const campaignPrivateStatus = (await fetch(`http://127.0.0.1:${campaignPort}/listening-analysis.json`)).status;
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
  const manualFallbackSessionId = JSON.parse(await page.locator("#manual-export").inputValue()).session_id;
  const analysisPath = join(downloads, "campaign-analysis.json");
  runPython(["scripts/dev/listening.py", "analyze", analysis, campaignSessionPath, "--out", analysisPath]);
  const campaignAnalysis = JSON.parse(await readFile(analysisPath, "utf8"));
  const errorCountBeforeMetadataFailure = errors.length;
  await page.route("**/*.wav", (route) => route.abort());
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForFunction(() => document.querySelector("#status")?.textContent.startsWith("Stored session retained but could not be verified"));
  const transientRecoveryStatus = await page.locator("#status").innerText();
  const transientMetadataFailureRetainedSession = await page.evaluate(
    (key) => localStorage.getItem(key) !== null && document.querySelector("#resume")?.hidden === true,
    `ij-listening:${campaignSession.session_id}`,
  );
  await page.unroute("**/*.wav");
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Recover completed session" }).waitFor();
  const expectedMetadataErrors = errors.splice(errorCountBeforeMetadataFailure);
  const metadataFailureWasExplicit = transientRecoveryStatus.startsWith("Stored session retained but could not be verified")
    && expectedMetadataErrors.every((message) => message.includes("net::ERR_FAILED"));
  await page.evaluate((valid) => {
    const corrupt = structuredClone(valid);
    corrupt.session_id = "corrupt-stored-session";
    corrupt.started_at = "9999-12-31T23:59:59.999Z";
    corrupt.trials[0].playback[corrupt.trials[0].presentation[0]].listened_ms = 0;
    localStorage.setItem("ij-listening:corrupt-stored-session", JSON.stringify(corrupt));
    localStorage.setItem("ij-listening:malformed-stored-session", "{not-json");
  }, campaignSession);
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Recover completed session" }).waitFor();
  const recoveryStatus = await page.locator("#status").innerText();
  const storedRecovery = await page.evaluate(() => ({
    corruptRemoved: localStorage.getItem("ij-listening:corrupt-stored-session") === null,
    malformedRemoved: localStorage.getItem("ij-listening:malformed-stored-session") === null,
  }));
  const campaignAssertions = {
    sampleRate44100: publicExperiment.sample_rate === 44100,
    participantManifestIsOpaque: !/candidate|incumbent|role|source_sha|222222/i.test(publicText),
    privateManifestNotServed: campaignPrivateStatus === 404,
    digestMatchesAcrossLanguages: campaignSession.experiment_digest === await manifestDigest(publicExperiment)
      && campaignSession.experiment_digest === campaignAnalysis.experiment_digest,
    resumeRecoveredCompletedTrial: firstProgress.includes("Trial 2 of 2") && resumedProgress.includes("Trial 2 of 2") && campaignSession.trials.length === 2,
    completePlaybackStored: campaignSession.trials.every((trial) => Object.values(trial.playback).every((item) => item.completed === 1 && item.listened_ms >= 790)),
    pythonAcceptedBrowserSession: campaignAnalysis.n_submitted === 1 && campaignAnalysis.n_included === 1,
    rawSessionRetained: campaignAnalysis.raw_sessions[0].session_id === campaignSession.session_id,
    manualFallbackPopulated: manualFallbackSessionId === campaignSession.session_id,
    corruptStoredSessionRejected: storedRecovery.corruptRemoved,
    malformedStoredSessionRejected: storedRecovery.malformedRemoved,
    invalidStorageWarningShown: recoveryStatus.startsWith("Discarded "),
    transientMetadataFailureRetainedSession,
    metadataFailureWasExplicit,
    noVerdict: campaignAnalysis.quality_verdict === null,
  };

  const abx = await createAbxBundle(candidate, bundle, analysis);
  const abxPort = port + 2;
  servers.push(spawn(PYTHON, ["-m", "http.server", String(abxPort), "--bind", "127.0.0.1"], { cwd: abx.bundle, stdio: "ignore" }));
  await waitForServer(`http://127.0.0.1:${abxPort}/`);
  await page.goto(`http://127.0.0.1:${abxPort}/?seed=2271560481`, { waitUntil: "networkidle" });
  const abxPublicText = JSON.stringify(abx.experiment);
  if (/x_source|candidate|incumbent/i.test(abxPublicText)) throw new Error("ABX participant manifest exposes its answer key");
  const abxPrivateStatus = (await fetch(`http://127.0.0.1:${abxPort}/listening-abx-analysis.json`)).status;
  await completeSetup(page, "abx-browser-pilot");
  for (let trial = 0; trial < abx.experiment.trials.length; trial++) {
    await playEveryVisibleSampleToCompletion(page);
    await page.getByRole("button", { name: "Sample 1", exact: true }).click();
    await page.getByRole("button", { name: trial + 1 === abx.experiment.trials.length ? "Submit session" : "Save and continue" }).click();
  }
  const abxSessionPath = join(downloads, "abx-session.json");
  const abxSession = await exportSession(page, abxSessionPath);
  const abxAnalysisPath = join(downloads, "abx-analysis.json");
  runPython(["scripts/dev/listening.py", "analyze", abx.analysis, abxSessionPath, "--out", abxAnalysisPath]);
  const abxAnalysis = JSON.parse(await readFile(abxAnalysisPath, "utf8"));
  const abxAssertions = {
    publicAnswerKeyAbsent: !/x_source|candidate|incumbent/i.test(abxPublicText),
    privateAnswerKeyNotServed: abxPrivateStatus === 404,
    opaqueXPlayedCompletely: abxSession.trials.every((trial) => trial.playback.x.completed === 1),
    pythonAcceptedBrowserSession: abxAnalysis.n_included === 1 && abxAnalysis.abx.total === abx.experiment.trials.length,
    noVerdict: abxAnalysis.quality_verdict === null,
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
  const rejectedTampering = [];
  const expectRestoreRejected = async (mutate) => {
    const changed = structuredClone(interruptedJson);
    mutate(changed);
    await fallbackPage.locator("#restore-json").fill(JSON.stringify(changed));
    await fallbackPage.locator("#status").evaluate((element) => { element.textContent = ""; });
    await fallbackPage.getByRole("button", { name: "Restore session JSON" }).click();
    await fallbackPage.waitForFunction(() => document.querySelector("#status")?.textContent.startsWith("Session restore failed:"));
    rejectedTampering.push((await fallbackPage.locator("#status").innerText()).startsWith("Session restore failed:") && await fallbackPage.locator("#setup").isVisible());
  };
  await expectRestoreRejected((value) => { value.randomization.seed = 4294967296; });
  await expectRestoreRejected((value) => { value.trials[0].presentation.reverse(); });
  await expectRestoreRejected((value) => { value.trials[0].playback[value.trials[0].presentation[0]].listened_ms = 0; });
  await expectRestoreRejected((value) => { value.trials[0].response.choice = "not-a-condition"; });
  await fallbackPage.locator("#restore-json").fill(interruptedJsonText);
  await fallbackPage.getByRole("button", { name: "Restore session JSON" }).click();
  const restoredProgress = await fallbackPage.locator(".progress").innerText();
  await answerAbTrial(fallbackPage);
  await fallbackPage.getByRole("button", { name: "Submit session" }).click();
  const fallbackJson = JSON.parse(await fallbackPage.locator("#manual-export").inputValue());
  const fallbackStatus = await fallbackPage.locator("#status").innerText();
  const fallbackAssertions = {
    inProgressCopyAvailable: exportVisibleDuringProgress && interruptedJson.trials.length === 1 && !interruptedJson.submitted_at,
    tamperedCopiesRejected: rejectedTampering.length === 4 && rejectedTampering.every(Boolean),
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
    abx: abxAssertions,
    storageFallback: fallbackAssertions,
  };
  const verdict = errors.length === 0
    && Object.values(pilotAssertions).every(Boolean)
    && Object.values(campaignAssertions).every(Boolean)
    && Object.values(abxAssertions).every(Boolean)
    && Object.values(fallbackAssertions).every(Boolean) ? "PASS" : "FAIL";
  console.log(JSON.stringify({ verdict, browser: BROWSER_NAME, seed, pilotPresentation: expected, assertions, errors }, null, 2));
  if (verdict !== "PASS") process.exitCode = 1;
} finally {
  if (browser) await browser.close();
  for (const child of servers) child.kill("SIGTERM");
  await rm(downloads, { recursive: true, force: true });
  await rm(campaignRoot, { recursive: true, force: true });
}
