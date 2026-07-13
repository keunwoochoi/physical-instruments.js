import { RANDOMIZATION_ALGORITHM, manifestDigest, presentations, trialOrder } from "./randomization.js";

const $ = (selector) => document.querySelector(selector);
const configuredExperiment = document.querySelector('meta[name="ij-listening-experiment"]')?.content;
const manifestUrl = new URLSearchParams(location.search).get("experiment") ?? configuredExperiment ?? "pilot/experiment.json";
let experiment = null, digest = null, base = null, byTrial = null;
try {
  experiment = await fetch(manifestUrl).then((response) => {
    if (!response.ok) throw new Error(`experiment load failed: ${response.status}`);
    return response.json();
  });
  digest = await manifestDigest(experiment);
  base = new URL(manifestUrl, location.href);
  byTrial = Object.fromEntries(experiment.trials.map((trial) => [trial.id, trial]));
} catch (error) {
  $("#title").textContent = "Listening experiment unavailable";
  $("#instructions").textContent = `The experiment could not be loaded or validated (${error.message}). Check the experiment link and ask the owner for a fresh bundle.`;
  $("#setup").hidden = true;
  $("#status").textContent = "No response data was collected.";
}

if (experiment) {
let session = null;
let recoverableSession = null;
let trialIndex = 0;
let activeAudio = null;
let storageAvailable = true;

class RetryableRecoveryError extends Error {}

$("#title").textContent = experiment.title;
$("#instructions").textContent = experiment.instructions;

function audioUrl(path) {
  return new URL(path, base).href;
}

function playCounts(order) {
  return Object.fromEntries(order.map((id) => [id, 0]));
}

function playbackEvidence(order) {
  return Object.fromEntries(order.map((id) => [id, { starts: 0, completed: 0, listened_ms: 0 }]));
}

function setStatus(message) {
  $("#status").textContent = message;
}

function sessionJson() {
  return session ? `${JSON.stringify(session, null, 2)}\n` : "";
}

function refreshManualExport() {
  $("#manual-export").value = sessionJson();
}

function exposeManualExport() {
  refreshManualExport();
  $("#manual-export-section").hidden = false;
}

function persist() {
  if (!session) return;
  if (!storageAvailable) {
    exposeManualExport();
    return;
  }
  try {
    localStorage.setItem(`ij-listening:${session.session_id}`, JSON.stringify(session));
  } catch (error) {
    storageAvailable = false;
    exposeManualExport();
    setStatus(`Browser storage is unavailable (${error.name}). The session remains in memory; use export or manual copy before leaving.`);
  }
}

async function recoverStoredSession() {
  try {
    const matches = [];
    const keys = [];
    for (let index = 0; index < localStorage.length; index++) {
      const key = localStorage.key(index);
      if (key?.startsWith("ij-listening:")) keys.push(key);
    }
    for (const key of keys) {
      try {
        const value = JSON.parse(localStorage.getItem(key));
        if (value.experiment_id === experiment.id && value.experiment_digest === digest) matches.push({ key, value });
      } catch {
        localStorage.removeItem(key);
        setStatus("Discarded malformed stored session. Use a known-good manual recovery copy if available.");
      }
    }
    matches.sort((a, b) => String(b.value.started_at).localeCompare(String(a.value.started_at)));
    for (const entry of matches) {
      try {
        return await validateRestoredSession(entry.value);
      } catch (error) {
        if (error instanceof RetryableRecoveryError) {
          setStatus(`Stored session retained but could not be verified (${error.message}). Restore connectivity and reload to retry.`);
          continue;
        }
        localStorage.removeItem(entry.key);
        setStatus(`Discarded invalid stored session (${error.message}). Use a known-good manual recovery copy if available.`);
      }
    }
    return null;
  } catch (error) {
    storageAvailable = false;
    setStatus(`Browser storage cannot be read (${error.name}). New sessions still work in memory and can be copied manually.`);
    return null;
  }
}

function stopActiveAudio(except = null) {
  if (activeAudio && activeAudio !== except) {
    activeAudio.pause();
    activeAudio.currentTime = 0;
  }
  if (activeAudio !== except) activeAudio = null;
}

function player(label, id, path, response) {
  const wrapper = document.createElement("div");
  wrapper.className = "sample";
  wrapper.innerHTML = `<header><strong>${label}</strong><span class="plays">0 complete plays</span></header><audio preload="metadata"></audio><button type="button" class="play">Play from start</button>`;
  const audio = wrapper.querySelector("audio");
  const button = wrapper.querySelector("button.play");
  audio.src = audioUrl(path);
  audio.volume = 1;
  button.setAttribute("aria-label", `Play ${label} from start`);
  button.addEventListener("click", async () => {
    stopActiveAudio(audio);
    audio.pause();
    audio.currentTime = 0;
    audio.volume = 1;
    activeAudio = audio;
    try {
      await audio.play();
    } catch (error) {
      setStatus(`Playback failed: ${error.message}`);
    }
  });
  audio.addEventListener("play", () => {
    response.play_counts[id] += 1;
    response.playback[id].starts += 1;
    persist();
  });
  audio.addEventListener("ended", () => {
    response.playback[id].completed += 1;
    response.playback[id].listened_ms += Math.round(audio.duration * 1000);
    wrapper.querySelector(".plays").textContent = `${response.playback[id].completed} complete plays`;
    if (activeAudio === audio) activeAudio = null;
    persist();
  });
  return wrapper;
}

function baseResponse(trial, order) {
  const slots = [...order];
  if (trial.protocol === "mushra") slots.push("reference");
  if (trial.protocol === "abx") slots.push("x");
  return {
    trial_id: trial.id,
    protocol: trial.protocol,
    presentation: order,
    response: {},
    play_counts: playCounts(slots),
    playback: playbackEvidence(slots),
  };
}

function showTrial() {
  stopActiveAudio();
  if (trialIndex >= experiment.trials.length) return finish();
  const trial = byTrial[session.trial_order[trialIndex]];
  const order = presentations(experiment, session.randomization.seed)[trial.id];
  const response = baseResponse(trial, order);
  const section = $("#trial");
  section.replaceChildren();
  section.hidden = false;
  const heading = document.createElement("div");
  const progress = document.createElement("p");
  progress.className = "progress";
  progress.textContent = `Trial ${trialIndex + 1} of ${experiment.trials.length} · ${trial.protocol.toUpperCase()}`;
  const prompt = document.createElement("h2");
  prompt.textContent = trial.prompt;
  heading.append(progress, prompt);
  section.append(heading);
  const players = document.createElement("div");
  players.className = "players";
  const stimulus = Object.fromEntries(trial.stimuli.map((item) => [item.id, item]));
  if (trial.protocol === "mushra") players.append(player("Explicit reference", "reference", trial.reference.path, response));
  order.forEach((id, index) => {
    const item = stimulus[id];
    const sample = player(`Sample ${index + 1}`, id, item.path, response);
    if (trial.protocol === "mushra") {
      const rating = document.createElement("label");
      rating.className = "rating";
      rating.innerHTML = `<span>0</span><input type="range" min="0" max="100" value="50"><output>—</output>`;
      const input = rating.querySelector("input"), output = rating.querySelector("output");
      input.addEventListener("input", () => {
        response.response.ratings ??= {};
        response.response.ratings[id] = Number(input.value);
        output.value = input.value;
        persist();
      });
      sample.append(rating);
    }
    players.append(sample);
  });
  if (trial.protocol === "abx") {
    players.append(player("X", "x", trial.x.path, response));
  }
  section.append(players);
  const choices = document.createElement("div");
  choices.className = "choices";
  if (trial.protocol !== "mushra") {
    const options = order.map((id, index) => ({ id, label: `Sample ${index + 1}` }));
    if (trial.protocol === "ab") options.push({ id: "tie", label: "No preference" });
    options.forEach((option) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = option.label;
      button.addEventListener("click", () => {
        response.response.choice = option.id;
        choices.querySelectorAll("button").forEach((item) => item.classList.remove("selected"));
        button.classList.add("selected");
        persist();
      });
      choices.append(button);
    });
    section.append(choices);
  }
  const next = document.createElement("button");
  next.textContent = trialIndex + 1 === experiment.trials.length ? "Submit session" : "Save and continue";
  next.addEventListener("click", () => {
    const answered = trial.protocol === "mushra"
      ? Object.keys(response.response.ratings ?? {}).length === order.length
      : Boolean(response.response.choice);
    const listened = Object.values(response.playback).every(
      (evidence) => evidence.completed >= experiment.exclusion_policy.min_completed_plays_per_stimulus,
    );
    if (!answered || !listened) return setStatus("Play every sample through to completion and complete every rating or choice before continuing.");
    session.trials.push(response);
    trialIndex += 1;
    persist();
    showTrial();
  });
  section.append(next);
}

function finish() {
  stopActiveAudio();
  if (!session.submitted_at) session.submitted_at = new Date().toISOString();
  persist();
  $("#trial").hidden = true;
  $("#complete").hidden = false;
  exposeManualExport();
  setStatus(storageAvailable
    ? "Session stored locally. Export the raw JSON to preserve it outside this browser."
    : "Session is complete in memory. Browser storage is unavailable; export or copy the raw JSON before leaving.");
}

function beginSession(value) {
  session = value;
  trialIndex = session.trials.length;
  $("#setup").hidden = true;
  $("#resume").hidden = true;
  persist();
  if (session.submitted_at) finish(); else showTrial();
}

function plainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireExactKeys(value, keys, label) {
  if (!plainObject(value) || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) throw new Error(`${label} fields do not match the session contract`);
}

function requireString(value, label, allowEmpty = true) {
  if (typeof value !== "string" || (!allowEmpty && !value)) throw new Error(`${label} must be ${allowEmpty ? "a string" : "a non-empty string"}`);
}

const durationCache = new Map();

function durationMs(path) {
  const url = audioUrl(path);
  if (!durationCache.has(url)) {
    const pending = new Promise((resolve, reject) => {
      const audio = new Audio();
      const timeout = setTimeout(() => reject(new RetryableRecoveryError("audio metadata timed out")), 10000);
      audio.preload = "metadata";
      audio.addEventListener("loadedmetadata", () => {
        clearTimeout(timeout);
        if (!Number.isFinite(audio.duration) || audio.duration <= 0) reject(new RetryableRecoveryError("audio duration is unavailable"));
        else resolve(Math.round(audio.duration * 1000));
      }, { once: true });
      audio.addEventListener("error", () => {
        clearTimeout(timeout);
        reject(new RetryableRecoveryError("audio metadata could not be loaded"));
      }, { once: true });
      audio.src = url;
    }).catch((error) => {
      durationCache.delete(url);
      throw error;
    });
    durationCache.set(url, pending);
  }
  return durationCache.get(url);
}

async function validateRestoredTrial(response, trial, expectedPresentation) {
  requireExactKeys(response, ["trial_id", "protocol", "presentation", "response", "play_counts", "playback"], `${trial.id} response`);
  if (response.trial_id !== trial.id || response.protocol !== trial.protocol || JSON.stringify(response.presentation) !== JSON.stringify(expectedPresentation)) throw new Error(`${trial.id} presentation or protocol was changed`);
  const slots = [...expectedPresentation];
  if (trial.protocol === "mushra") slots.push("reference");
  if (trial.protocol === "abx") slots.push("x");
  const slotSet = new Set(slots);
  if (slotSet.size !== slots.length) throw new Error(`${trial.id} playback slots are not unique`);
  requireExactKeys(response.play_counts, slots, `${trial.id} play counts`);
  requireExactKeys(response.playback, slots, `${trial.id} playback evidence`);
  const stimulusPaths = Object.fromEntries(trial.stimuli.map((item) => [item.id, item.path]));
  if (trial.protocol === "mushra") stimulusPaths.reference = trial.reference.path;
  if (trial.protocol === "abx") stimulusPaths.x = trial.x.path;
  for (const slot of slots) {
    const count = response.play_counts[slot];
    const evidence = response.playback[slot];
    if (!Number.isInteger(count) || count < 0) throw new Error(`${trial.id} play count is invalid`);
    requireExactKeys(evidence, ["starts", "completed", "listened_ms"], `${trial.id} playback item`);
    if (![evidence.starts, evidence.completed, evidence.listened_ms].every((item) => Number.isInteger(item) && item >= 0)
      || evidence.starts !== count || evidence.completed > evidence.starts
      || evidence.completed < experiment.exclusion_policy.min_completed_plays_per_stimulus) throw new Error(`${trial.id} playback evidence is invalid`);
    const expectedListenedMs = await durationMs(stimulusPaths[slot]) * evidence.completed;
    if (evidence.listened_ms < 0.95 * expectedListenedMs) throw new Error(`${trial.id} playback duration coverage is invalid`);
  }
  if (trial.protocol === "mushra") {
    requireExactKeys(response.response, ["ratings"], `${trial.id} answer`);
    requireExactKeys(response.response.ratings, expectedPresentation, `${trial.id} ratings`);
    if (!Object.values(response.response.ratings).every((score) => Number.isInteger(score) && score >= 0 && score <= 100)) throw new Error(`${trial.id} ratings are invalid`);
  } else {
    requireExactKeys(response.response, ["choice"], `${trial.id} answer`);
    const choices = trial.protocol === "ab" ? [...expectedPresentation, "tie"] : expectedPresentation;
    if (!choices.includes(response.response.choice)) throw new Error(`${trial.id} choice is invalid`);
  }
}

async function validateRestoredSession(value) {
  requireExactKeys(value, ["schema_version", "experiment_id", "experiment_digest", "session_id", "evidence_kind", "listener", "setup", "randomization", "trial_order", "started_at", "submitted_at", "trials"], "session");
  if (value.schema_version !== "1.0.0" || value.evidence_kind !== "human") throw new Error("session schema version or evidence kind is invalid");
  if (value.experiment_id !== experiment.id || value.experiment_digest !== digest) throw new Error("session belongs to a different experiment or manifest");
  requireString(value.session_id, "session ID", false);
  requireString(value.started_at, "start time");
  requireString(value.submitted_at, "submission time");
  requireExactKeys(value.listener, ["id", "experience", "hearing_notes"], "listener");
  requireString(value.listener.id, "listener ID", false);
  requireString(value.listener.experience, "listener experience");
  requireString(value.listener.hearing_notes, "hearing notes");
  requireExactKeys(value.setup, ["transducer", "environment", "device", "volume_check"], "setup");
  if (!["headphones", "studio_monitors", "speakers", "other"].includes(value.setup.transducer)
    || typeof value.setup.environment !== "string" || !value.setup.environment
    || typeof value.setup.device !== "string" || !value.setup.device
    || value.setup.volume_check !== true) throw new Error("session setup is invalid");
  requireExactKeys(value.randomization, ["algorithm", "seed"], "randomization");
  if (value.randomization.algorithm !== RANDOMIZATION_ALGORITHM || !Number.isInteger(value.randomization.seed)
    || value.randomization.seed < 0 || value.randomization.seed > 0xFFFFFFFF) throw new Error("session randomization is invalid");
  const expectedOrder = trialOrder(experiment, value.randomization.seed);
  if (JSON.stringify(value.trial_order) !== JSON.stringify(expectedOrder)) throw new Error("session trial order does not match its sealed seed");
  if (!Array.isArray(value.trials) || value.trials.length > expectedOrder.length) throw new Error("session trial evidence is incomplete or malformed");
  const expectedPresentations = presentations(experiment, value.randomization.seed);
  for (let index = 0; index < value.trials.length; index++) await validateRestoredTrial(value.trials[index], byTrial[expectedOrder[index]], expectedPresentations[expectedOrder[index]]);
  if (value.submitted_at && value.trials.length !== expectedOrder.length) throw new Error("submitted session is missing trials");
  return value;
}

$("#start").addEventListener("click", () => {
  const listener = $("#listener").value.trim();
  const experience = $("#experience").value.trim();
  const environment = $("#environment").value.trim();
  const device = $("#device").value.trim();
  if (!listener || !experience || !environment || !device || !$("#volume").checked) return setStatus("Complete the setup and fixed-volume check first.");
  const forcedSeed = new URLSearchParams(location.search).get("seed");
  const seed = forcedSeed === null ? crypto.getRandomValues(new Uint32Array(1))[0] : Number(forcedSeed) >>> 0;
  beginSession({
    schema_version: "1.0.0",
    experiment_id: experiment.id,
    experiment_digest: digest,
    session_id: `${experiment.id}-${Date.now()}-${seed.toString(16).padStart(8, "0")}`,
    evidence_kind: "human",
    listener: { id: listener, experience, hearing_notes: $("#hearing").value.trim() },
    setup: { transducer: $("#transducer").value, environment, device, volume_check: true },
    randomization: { algorithm: RANDOMIZATION_ALGORITHM, seed },
    trial_order: trialOrder(experiment, seed),
    started_at: new Date().toISOString(),
    submitted_at: "",
    trials: [],
  });
});

$("#resume").addEventListener("click", () => beginSession(recoverableSession));

$("#restore").addEventListener("click", async () => {
  try {
    const restored = await validateRestoredSession(JSON.parse($("#restore-json").value));
    beginSession(restored);
    setStatus(restored.submitted_at ? "Completed session restored from manual JSON." : `In-progress session restored at trial ${restored.trials.length + 1}.`);
  } catch (error) {
    setStatus(`Session restore failed: ${error.message}`);
  }
});

$("#download").addEventListener("click", () => {
  refreshManualExport();
  try {
    const blob = new Blob([sessionJson()], { type: "application/json" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = `${session.session_id}.json`;
    document.body.append(link);
    link.click();
    setTimeout(() => {
      URL.revokeObjectURL(link.href);
      link.remove();
    }, 1000);
  } catch (error) {
    setStatus(`Automatic download failed (${error.message}). Copy the raw JSON shown below.`);
  }
});

$("#copy").addEventListener("click", async () => {
  refreshManualExport();
  try {
    await navigator.clipboard.writeText($("#manual-export").value);
    setStatus("Raw session JSON copied to the clipboard.");
  } catch {
    $("#manual-export").focus();
    $("#manual-export").select();
    setStatus("Clipboard access is unavailable. The raw JSON is selected for manual copy.");
  }
});

$("#clear").addEventListener("click", () => {
  if (session && storageAvailable) {
    try { localStorage.removeItem(`ij-listening:${session.session_id}`); } catch {}
  }
  location.reload();
});

recoverableSession = await recoverStoredSession();
if (recoverableSession) {
  $("#resume").hidden = false;
  $("#resume").textContent = recoverableSession.submitted_at ? "Recover completed session" : `Resume saved session (${recoverableSession.trials.length}/${experiment.trials.length})`;
}
}
