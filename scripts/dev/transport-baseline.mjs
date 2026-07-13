#!/usr/bin/env node
/** Freeze and verify the no-control render contract before event transport moves into WASM. */
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const CONTRACT_PATH = new URL("../../evals/transport/no-control-contract-v1.json", import.meta.url);

function stableJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function rootUrl(relativePath) {
  return new URL(relativePath, `file://${ROOT.endsWith("/") ? ROOT : `${ROOT}/`}`);
}

function validateContract(contract, cases) {
  const matrix = contract.matrix;
  const exact = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${label} must remain ${JSON.stringify(expected)}`);
  };
  exact(matrix.delivery_simulations, ["quantum-delivery", "preloaded"], "delivery_simulations");
  exact(matrix.sample_rates, [44100, 48000], "sample_rates");
  exact(matrix.event_offsets, [0, 1, 63, 127], "event_offsets");
  exact(matrix.process_chunk_sizes, [1, 17, 64, 127, 128], "process_chunk_sizes");
  if (contract.metric.version !== "transport-drift-rms-v1") throw new Error("unknown transport metric version");
  if (!(contract.metric.threshold > 0 && contract.metric.threshold <= 1e-4)) throw new Error("transport drift threshold must be in (0, 1e-4]");
  if (!Number.isInteger(contract.metric.fingerprint_blocks) || contract.metric.fingerprint_blocks < 16) throw new Error("fingerprint_blocks must be an integer >= 16");
  if (!Array.isArray(cases.cases) || cases.cases.length === 0) throw new Error("no-control case manifest is empty");
  const ids = new Set();
  const eventPriority = { track: 0, room: 1, reverb: 2, pedal: 3, off: 3, on: 4 };
  for (const item of cases.cases) {
    if (!item.id || ids.has(item.id)) throw new Error(`invalid or duplicate case id: ${item.id}`);
    ids.add(item.id);
    if (!Number.isInteger(item.total_frames) || item.total_frames <= 0) throw new Error(`${item.id}: total_frames must be positive`);
    let prior = -1;
    let priorPriority = -1;
    for (const event of item.events) {
      if (!Number.isInteger(event.frame) || event.frame < prior) throw new Error(`${item.id}: events must be frame-sorted`);
      if (!(event.kind in eventPriority)) throw new Error(`${item.id}: unsupported event kind ${event.kind}`);
      if (event.frame !== prior) priorPriority = -1;
      if (eventPriority[event.kind] < priorPriority) throw new Error(`${item.id}: equal-frame events violate the frozen priority order at frame ${event.frame}`);
      prior = event.frame;
      priorPriority = eventPriority[event.kind];
      if (event.frame + (event.offset ? Math.max(...matrix.event_offsets) : 0) >= item.total_frames) throw new Error(`${item.id}: event exceeds render extent`);
    }
  }
}

function eventFrame(event, offset) {
  return event.frame + (event.offset ? offset : 0);
}

function applyEvent(x, engine, event) {
  switch (event.kind) {
    case "track": x.ij_set_track(engine, event.track, event.instrument, event.gain, event.pan); break;
    case "room": x.ij_set_room(engine, event.track, event.send); break;
    case "reverb": x.ij_set_reverb(engine, event.reverb); break;
    case "pedal": x.ij_pedal(engine, event.track, event.on); break;
    case "on": x.ij_note_on(engine, event.track, event.midi, event.velocity); break;
    case "off": x.ij_note_off(engine, event.track, event.midi); break;
    default: throw new Error(`unsupported event kind: ${event.kind}`);
  }
}

function appendSamples(targetL, targetR, frame, x, engine, count) {
  x.ij_process(engine, count);
  const memory = x.memory.buffer;
  targetL.set(new Float32Array(memory, x.ij_out_l(engine), count), frame);
  targetR.set(new Float32Array(memory, x.ij_out_r(engine), count), frame);
}

async function renderCell(wasm, item, mode, sampleRate, offset, chunkSize, deliveryQuantum) {
  const { instance } = await WebAssembly.instantiate(wasm, {});
  const x = instance.exports;
  const engine = x.ij_engine_new(sampleRate);
  const left = new Float32Array(item.total_frames);
  const right = new Float32Array(item.total_frames);
  const source = item.events
    .map((event, sequence) => ({ ...event, sequence, resolvedFrame: eventFrame(event, offset) }))
    .sort((a, b) => a.resolvedFrame - b.resolvedFrame || a.sequence - b.sequence);
  const queue = mode === "preloaded" ? source.slice() : [];
  let sourceHead = mode === "preloaded" ? source.length : 0;
  let queueHead = 0;
  let frame = 0;
  let nextDelivery = 0;
  try {
    while (frame < item.total_frames) {
      if (mode === "quantum-delivery" && frame === nextDelivery) {
        const deliveryEnd = nextDelivery + deliveryQuantum;
        while (sourceHead < source.length && source[sourceHead].resolvedFrame < deliveryEnd) queue.push(source[sourceHead++]);
        nextDelivery = deliveryEnd;
      }
      while (queueHead < queue.length && queue[queueHead].resolvedFrame <= frame) applyEvent(x, engine, queue[queueHead++]);
      let boundary = Math.min(item.total_frames, frame + chunkSize);
      if (queueHead < queue.length) boundary = Math.min(boundary, queue[queueHead].resolvedFrame);
      if (mode === "quantum-delivery") boundary = Math.min(boundary, nextDelivery);
      if (boundary <= frame) throw new Error(`renderer made no progress at frame ${frame}`);
      const count = boundary - frame;
      appendSamples(left, right, frame, x, engine, count);
      frame = boundary;
    }
  } finally {
    x.ij_engine_free(engine);
  }
  return { left, right };
}

function pcmBytes(left, right) {
  const bytes = new Uint8Array((left.length + right.length) * 4);
  const view = new DataView(bytes.buffer);
  let cursor = 0;
  for (let i = 0; i < left.length; i++) {
    view.setFloat32(cursor, left[i], true); cursor += 4;
    view.setFloat32(cursor, right[i], true); cursor += 4;
  }
  return bytes;
}

function summarize(left, right, blockCount, artifactPolicy) {
  let sumSquares = 0;
  let peak = 0;
  for (let i = 0; i < left.length; i++) {
    const l = left[i], r = right[i];
    if (!Number.isFinite(l) || !Number.isFinite(r)) throw new Error(`non-finite sample at frame ${i}`);
    sumSquares += l * l + r * r;
    peak = Math.max(peak, Math.abs(l), Math.abs(r));
  }
  const rms = Math.sqrt(sumSquares / (left.length * 2));
  if (rms < artifactPolicy.minimum_rms) throw new Error(`silent render: rms=${rms}`);
  if (peak > artifactPolicy.maximum_absolute_peak) throw new Error(`unbounded render: peak=${peak}`);
  const fingerprint = [];
  for (let block = 0; block < blockCount; block++) {
    const start = Math.floor(block * left.length / blockCount);
    const end = Math.floor((block + 1) * left.length / blockCount);
    let meanL = 0, meanR = 0, squareL = 0, squareR = 0;
    for (let i = start; i < end; i++) {
      meanL += left[i]; meanR += right[i]; squareL += left[i] * left[i]; squareR += right[i] * right[i];
    }
    const count = Math.max(1, end - start);
    fingerprint.push(meanL / count, meanR / count, Math.sqrt(squareL / count), Math.sqrt(squareR / count));
  }
  return { rms, peak, fingerprint };
}

function compactNumber(value) {
  return Number(value.toPrecision(12));
}

function encodeFingerprint(values) {
  const array = Float32Array.from(values);
  return Buffer.from(array.buffer, array.byteOffset, array.byteLength).toString("base64");
}

function decodeFingerprint(value) {
  const bytes = Buffer.from(value, "base64");
  if (bytes.byteLength % 4 !== 0) throw new Error("invalid baseline fingerprint encoding");
  const aligned = new Uint8Array(bytes);
  return Array.from(new Float32Array(aligned.buffer));
}

function compactSummary(summary) {
  return {
    rms: compactNumber(summary.rms),
    peak: compactNumber(summary.peak),
    fingerprint_f32le_base64: encodeFingerprint(summary.fingerprint),
  };
}

function drift(candidate, reference) {
  if (candidate.length !== reference.length) throw new Error("fingerprint length mismatch");
  let error = 0, base = 0;
  for (let i = 0; i < reference.length; i++) {
    const delta = candidate[i] - reference[i];
    error += delta * delta;
    base += reference[i] * reference[i];
  }
  return Math.sqrt(error / candidate.length) / Math.max(1e-12, Math.sqrt(base / reference.length));
}

function cellId(caseId, mode, sampleRate, offset, chunkSize) {
  return `${caseId}|${mode}|${sampleRate}|offset=${offset}|chunk=${chunkSize}`;
}

async function renderMatrix(wasm, contract, cases) {
  const cells = {};
  for (const item of cases.cases) {
    for (const mode of contract.matrix.delivery_simulations) {
      for (const sampleRate of contract.matrix.sample_rates) {
        for (const offset of contract.matrix.event_offsets) {
          for (const chunkSize of contract.matrix.process_chunk_sizes) {
            const rendered = await renderCell(wasm, item, mode, sampleRate, offset, chunkSize, contract.matrix.live_delivery_quantum_frames);
            const summary = summarize(rendered.left, rendered.right, contract.metric.fingerprint_blocks, contract.artifact_policy);
            const id = cellId(item.id, mode, sampleRate, offset, chunkSize);
            cells[id] = {
              pcm_sha256: sha256(pcmBytes(rendered.left, rendered.right)),
              rms: summary.rms,
              peak: summary.peak,
              fingerprint: summary.fingerprint,
            };
          }
        }
      }
    }
  }
  return cells;
}

function compactBaselineCells(cells, contract, cases) {
  const references = {};
  const matrix = {};
  for (const item of cases.cases) {
    for (const mode of contract.matrix.delivery_simulations) for (const sampleRate of contract.matrix.sample_rates) {
      for (const offset of contract.matrix.event_offsets) for (const chunkSize of contract.matrix.process_chunk_sizes) {
        const id = cellId(item.id, mode, sampleRate, offset, chunkSize);
        const cell = cells[id];
        references[id] = { pcm_sha256: cell.pcm_sha256, ...compactSummary(cell) };
        matrix[id] = { pcm_sha256: cell.pcm_sha256, reference: id };
      }
    }
  }
  return { references, matrix };
}

function compareRuns(first, second, contract) {
  const failures = [];
  for (const [id, reference] of Object.entries(first)) {
    const candidate = second[id];
    if (!candidate) { failures.push(`${id}: missing second-run cell`); continue; }
    if (candidate.pcm_sha256 !== reference.pcm_sha256) failures.push(`${id}: repeated run changed its exact PCM digest`);
    const distance = drift(candidate.fingerprint, reference.fingerprint);
    if (distance > contract.metric.threshold) failures.push(`${id}: repeat drift ${distance} > ${contract.metric.threshold}`);
  }
  if (failures.length) throw new Error(`two-run determinism failed:\n${failures.join("\n")}`);
}

function compareCandidate(cells, baseline, contract) {
  const failures = [];
  let maximumDrift = 0;
  let exactCells = 0;
  for (const [id, expected] of Object.entries(baseline.matrix)) {
    const candidate = cells[id];
    if (!candidate) { failures.push(`${id}: missing candidate cell`); continue; }
    const reference = baseline.references[expected.reference];
    const distance = drift(candidate.fingerprint, decodeFingerprint(reference.fingerprint_f32le_base64));
    maximumDrift = Math.max(maximumDrift, distance);
    if (candidate.pcm_sha256 === expected.pcm_sha256) exactCells++;
    else failures.push(`${id}: exact interleaved Float32 PCM digest changed`);
    if (distance > contract.metric.threshold) failures.push(`${id}: drift ${distance} > ${contract.metric.threshold}`);
  }
  if (failures.length) throw new Error(`transport-preservation candidate failed:\n${failures.slice(0, 20).join("\n")}${failures.length > 20 ? `\n... ${failures.length - 20} more` : ""}`);
  return { cells: Object.keys(baseline.matrix).length, exactCells, maximumDrift };
}

async function proveMutationGate(wasm, baseline, contract, cases) {
  const item = cases.cases[0];
  const mode = contract.matrix.delivery_simulations[0];
  const sampleRate = contract.matrix.sample_rates[0];
  const offset = contract.matrix.event_offsets[0];
  const chunkSize = contract.matrix.process_chunk_sizes[0];
  const key = cellId(item.id, mode, sampleRate, offset, chunkSize);
  const reference = baseline.references[key];
  const rendered = await renderCell(wasm, item, mode, sampleRate, offset, chunkSize, contract.matrix.live_delivery_quantum_frames);
  const identicalSummary = summarize(rendered.left, rendered.right, contract.metric.fingerprint_blocks, contract.artifact_policy);
  const referenceFingerprint = decodeFingerprint(reference.fingerprint_f32le_base64);
  const identical = drift(identicalSummary.fingerprint, referenceFingerprint);
  const identicalPcmSha256 = sha256(pcmBytes(rendered.left, rendered.right));
  const shiftedLeft = new Float32Array(rendered.left.length);
  const shiftedRight = new Float32Array(rendered.right.length);
  shiftedLeft.set(rendered.left.subarray(0, -1), 1);
  shiftedRight.set(rendered.right.subarray(0, -1), 1);
  const timingSummary = summarize(shiftedLeft, shiftedRight, contract.metric.fingerprint_blocks, contract.artifact_policy);
  const timingMutationDrift = drift(timingSummary.fingerprint, referenceFingerprint);
  const timingMutationPcmSha256 = sha256(pcmBytes(shiftedLeft, shiftedRight));
  for (let i = 0; i < rendered.left.length; i++) {
    rendered.left[i] *= 1.001;
    rendered.right[i] *= 1.001;
  }
  const mutatedSummary = summarize(rendered.left, rendered.right, contract.metric.fingerprint_blocks, contract.artifact_policy);
  const gainMutationDrift = drift(mutatedSummary.fingerprint, referenceFingerprint);
  if (identical > contract.metric.threshold) throw new Error(`identical fingerprint crossed threshold: ${identical}`);
  if (identicalPcmSha256 !== reference.pcm_sha256) throw new Error("identical render changed its exact PCM digest");
  if (timingMutationDrift <= contract.metric.threshold) throw new Error(`one-frame timing mutation did not cross threshold: ${timingMutationDrift}`);
  if (timingMutationPcmSha256 === reference.pcm_sha256) throw new Error("one-frame timing mutation retained the exact PCM digest");
  if (gainMutationDrift <= contract.metric.threshold) throw new Error(`synthetic gain mutation did not cross threshold: ${gainMutationDrift}`);
  return { reference: key, identicalDrift: identical, timingMutationDrift, gainMutationDrift, exactDigestRejectsTimingMutation: true };
}

async function identities(contract, casesBytes, wasmBytes) {
  const rendererBytes = await readFile(rootUrl(contract.renderer));
  const contractBytes = await readFile(CONTRACT_PATH);
  return {
    corpus_contract_sha256: sha256(contractBytes),
    case_manifest_sha256: sha256(casesBytes),
    eval_code_sha256: sha256(rendererBytes),
    shipped_wasm_sha256: sha256(wasmBytes),
  };
}

async function main() {
  const [command = "verify", flag] = process.argv.slice(2);
  if (!new Set(["generate", "verify"]).has(command)) throw new Error("usage: transport-baseline.mjs <generate|verify> [--allow-rebaseline]");
  if (command === "generate" && flag !== "--allow-rebaseline") throw new Error("generate requires --allow-rebaseline; baseline changes require separate review");
  const contractBytes = await readFile(CONTRACT_PATH);
  const contract = JSON.parse(contractBytes);
  const casePath = rootUrl(contract.case_manifest);
  const casesBytes = await readFile(casePath);
  const cases = JSON.parse(casesBytes);
  validateContract(contract, cases);
  const wasmPath = rootUrl(contract.shipped_wasm);
  const wasm = await readFile(wasmPath);
  const currentIdentities = await identities(contract, casesBytes, wasm);
  const first = await renderMatrix(wasm, contract, cases);
  const second = await renderMatrix(wasm, contract, cases);
  compareRuns(first, second, contract);

  if (command === "generate") {
    const compact = compactBaselineCells(first, contract, cases);
    const baseline = {
      schema_version: "1.0.0",
      contract_id: contract.contract_id,
      identities: currentIdentities,
      ...compact,
    };
    const mutationGate = await proveMutationGate(wasm, baseline, contract, cases);
    await writeFile(rootUrl(contract.baseline), stableJson(baseline));
    console.log(JSON.stringify({ verdict: "PASS", mode: "generated", cells: Object.keys(baseline.matrix).length, mutationGate }, null, 2));
    return;
  }

  const baseline = await readJson(rootUrl(contract.baseline));
  for (const key of ["corpus_contract_sha256", "case_manifest_sha256", "eval_code_sha256"]) {
    if (baseline.identities[key] !== currentIdentities[key]) throw new Error(`${key} changed; the frozen baseline is invalid and must be regenerated in a separately reviewed prerequisite`);
  }
  const result = compareCandidate(first, baseline, contract);
  const mutationGate = await proveMutationGate(wasm, baseline, contract, cases);
  const mode = baseline.identities.shipped_wasm_sha256 === currentIdentities.shipped_wasm_sha256 ? "reference" : "candidate";
  console.log(JSON.stringify({ verdict: "PASS", mode, ...result, twoRunDeterminism: "PASS", mutationGate, currentWasmSha256: currentIdentities.shipped_wasm_sha256, baselineWasmSha256: baseline.identities.shipped_wasm_sha256 }, null, 2));
}

main().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
