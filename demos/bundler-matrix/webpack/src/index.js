import { createEngine } from "instruments.js";
const status = (t) => (document.getElementById("status").textContent = t);
document.getElementById("start").addEventListener("click", async () => {
  try {
    const engine = await createEngine();
    await engine.ready;
    engine.createTrack("marimba").noteOn(69, 110);
    status("engine live");
  } catch (err) { status(`engine failed: ${err.message}`); console.error(err); }
});
