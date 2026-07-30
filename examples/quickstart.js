import { createEngine } from "physical-instruments.js";

const engine = await createEngine();          // lazy AudioContext, gesture-safe
const piano  = engine.createTrack("piano");
piano.noteOn(60, 96);                          // velocity changes timbre, not just volume
// --8<-- everything below is harness, not quickstart; the README block stops here
// scripts/verify/check-quickstart.mjs runs THIS FILE against the packed, installed
// package and taps engine.output — documented public surface — so the snippet above runs
// completely unmodified. The check also asserts the README contains it verbatim, so the
// text a reader copies and the text that is executed cannot drift apart.
export { engine };
