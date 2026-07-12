// The zero-config test: no workletUrl/wasmUrl overrides — the library's
// import.meta.url resolution must survive Vite dev AND build.
import { createEngine } from "instruments.js";

const status = (t) => (document.getElementById("status").textContent = t);
document.getElementById("start").addEventListener("click", async () => {
  try {
    const engine = await createEngine();
    await engine.ready;
    const t = engine.createTrack("marimba");
    t.noteOn(69, 110);
    status("engine live");
  } catch (err) {
    status(`engine failed: ${err.message}`);
    console.error(err);
  }
});
