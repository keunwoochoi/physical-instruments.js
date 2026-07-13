"use client";
import { useState } from "react";
import { createEngine } from "instruments.js";

// Zero-config test: SSR-safety (import at module level in a client component
// that still renders on the server) + import.meta.url asset resolution.
export default function Page() {
  const [status, setStatus] = useState("idle");
  return (
    <main>
      <button id="start" onClick={async () => {
        try {
          const engine = await createEngine();
          await engine.ready;
          engine.createTrack("marimba").noteOn(69, 110);
          setStatus("engine live");
        } catch (err) {
          setStatus(`engine failed: ${err.message}`);
          console.error(err);
        }
      }}>▶ Start</button>
      <div id="status">{status}</div>
    </main>
  );
}
