import { RANDOMIZATION_ALGORITHM, manifestDigest, presentations, trialOrder } from "./randomization.js";

const $ = (selector) => document.querySelector(selector);
const configuredExperiment = document.querySelector('meta[name="ij-listening-experiment"]')?.content;
const manifestUrl = new URLSearchParams(location.search).get("experiment") ?? configuredExperiment ?? "pilot/experiment.json";
const experiment = await fetch(manifestUrl).then((response) => {
  if (!response.ok) throw new Error(`experiment load failed: ${response.status}`);
  return response.json();
});
const digest = await manifestDigest(experiment);
const base = new URL(manifestUrl, location.href);
const byTrial = Object.fromEntries(experiment.trials.map((trial) => [trial.id, trial]));
let session = null;
let recoverableSession = null;
let trialIndex = 0;
let activeAudio = null;
let storageAvailable = true;

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

function recoverStoredSession() {
  try {
    const matches = [];
    for (let index = 0; index < localStorage.length; index++) {
      const key = localStorage.key(index);
      if (!key?.startsWith("ij-listening:")) continue;
      try {
        const value = JSON.parse(localStorage.getItem(key));
        if (value.experiment_id === experiment.id && value.experiment_digest === digest) matches.push(value);
      } catch {}
    }
    matches.sort((a, b) => String(b.started_at).localeCompare(String(a.started_at)));
    return matches[0] ?? null;
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
  heading.innerHTML = `<p class="progress">Trial ${trialIndex + 1} of ${experiment.trials.length} · ${trial.protocol.toUpperCase()}</p><h2>${trial.prompt}</h2>`;
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
    const xItem = stimulus[trial.x_source];
    players.append(player("X", "x", xItem.path, response));
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

function validateRestoredSession(value) {
  if (!value || typeof value !== "object") throw new Error("session must be a JSON object");
  if (value.experiment_id !== experiment.id || value.experiment_digest !== digest) throw new Error("session belongs to a different experiment or manifest");
  if (!Number.isInteger(value.randomization?.seed)) throw new Error("session randomization seed is missing");
  const expectedOrder = trialOrder(experiment, value.randomization.seed);
  if (JSON.stringify(value.trial_order) !== JSON.stringify(expectedOrder)) throw new Error("session trial order does not match its sealed seed");
  if (!Array.isArray(value.trials) || value.trials.length > expectedOrder.length) throw new Error("session trial evidence is incomplete or malformed");
  const completedIds = value.trials.map((trial) => trial?.trial_id);
  if (JSON.stringify(completedIds) !== JSON.stringify(expectedOrder.slice(0, completedIds.length))) throw new Error("completed trials are not a valid experiment prefix");
  if (!value.session_id || !value.listener?.id || !value.setup?.volume_check) throw new Error("session identity or setup evidence is missing");
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

$("#restore").addEventListener("click", () => {
  try {
    const restored = validateRestoredSession(JSON.parse($("#restore-json").value));
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

recoverableSession = recoverStoredSession();
if (recoverableSession) {
  $("#resume").hidden = false;
  $("#resume").textContent = recoverableSession.submitted_at ? "Recover completed session" : `Resume saved session (${recoverableSession.trials.length}/${experiment.trials.length})`;
}
