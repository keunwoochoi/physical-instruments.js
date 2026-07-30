"use client";
import { useEffect, useState } from "react";

// Web Audio is a browser API, so this must be a client component and the import must be
// dynamic -- Next evaluates module scope on the server during the build, and a top-level
// import of an AudioWorklet-based library would run there. That is the one Next-specific
// thing in these fixtures, and it is a fact about Next, not a workaround for us.
export default function Page() {
  const [state, setState] = useState("running");
  useEffect(() => {
    (async () => {
      const { run } = await import("../app.js");
      window.__result = run();
      setState("started");
    })();
  }, []);
  return <main>{state}</main>;
}
