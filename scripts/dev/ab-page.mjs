#!/usr/bin/env node
/**
 * Build a local A/B listening page from two standardized-audition dirs
 * (loop audit 2026-07-12 — the human gate's time is the loop's scarcest
 * resource; make listening passes fast and diffable).
 *
 *   node scripts/dev/ab-page.mjs <dirA(old)> <dirB(new)> [out.html]
 *
 * Pairs files by identical filename. "Blind" toggle hides labels and shuffles
 * A/B order per row (seeded per page load).
 */
import { readdir, writeFile } from "node:fs/promises";
import { resolve, relative, dirname } from "node:path";

const [dirA, dirB, outArg] = process.argv.slice(2);
if (!dirA || !dirB) { console.error("usage: ab-page.mjs <dirA(old)> <dirB(new)> [out.html]"); process.exit(1); }
const out = resolve(outArg ?? "ab-listen.html");
const A = resolve(dirA), B = resolve(dirB);
const fa = new Set((await readdir(A)).filter((f) => f.endsWith(".wav")));
const fb = (await readdir(B)).filter((f) => f.endsWith(".wav"));
const pairs = fb.filter((f) => fa.has(f)).sort();
const onlyB = fb.filter((f) => !fa.has(f)).sort();
const rel = (d, f) => relative(dirname(out), resolve(d, f));

const rows = pairs.map((f, i) => `
  <tr data-i="${i}">
    <td class="name">${f.replace(".wav", "")}</td>
    <td class="a"><span class="tag">A·old</span><audio controls preload="none" src="${rel(A, f)}"></audio></td>
    <td class="b"><span class="tag">B·new</span><audio controls preload="none" src="${rel(B, f)}"></audio></td>
    <td class="verdict"><button data-v="A">A</button><button data-v="~">~</button><button data-v="B">B</button></td>
  </tr>`).join("");

const html = `<!doctype html><meta charset="utf-8"><title>instruments.js A/B</title>
<style>
  body{font:14px system-ui;margin:2rem auto;max-width:70rem;background:#191512;color:#eee}
  table{border-collapse:collapse;width:100%} td{padding:.45rem .6rem;border-bottom:1px solid #333}
  .name{font-family:ui-monospace,monospace;font-size:.8rem;opacity:.85}
  .tag{font-size:.65rem;opacity:.6;display:block} audio{width:230px;height:32px}
  .blind .tag{visibility:hidden} .blind .name{visibility:hidden}
  tr.swap .a{transform:translateX(calc(100% + 1.2rem))} tr.swap .b{transform:translateX(calc(-100% - 1.2rem))}
  td.a,td.b{transition:none;position:relative}
  .verdict button{margin:0 2px;padding:.3rem .7rem;background:#2a241f;color:#eee;border:1px solid #444;border-radius:4px;cursor:pointer}
  .verdict button.sel{background:#b45525;border-color:#b45525}
  #bar{margin-bottom:1rem;display:flex;gap:1rem;align-items:center}
  #summary{font-family:ui-monospace,monospace;font-size:.85rem;opacity:.9}
</style>
<div id="bar">
  <h2 style="margin:0">A/B — ${pairs.length} pairs</h2>
  <label><input type="checkbox" id="blind"> blind (hide labels, shuffle sides)</label>
  <span id="summary"></span>
</div>
<table id="t">${rows}</table>
${onlyB.length ? `<h3>New-only (no counterpart in A)</h3>` + onlyB.map((f) => `<div>${f.replace(".wav", "")} <audio controls preload="none" src="${rel(B, f)}"></audio></div>`).join("") : ""}
<script>
  const votes = {};
  const swapped = new Set();
  document.getElementById("blind").addEventListener("change", (e) => {
    document.body.classList.toggle("blind", e.target.checked);
    document.querySelectorAll("#t tr").forEach((tr) => {
      const i = tr.dataset.i;
      if (e.target.checked && Math.random() < 0.5) { tr.classList.add("swap"); swapped.add(i); }
      else { tr.classList.remove("swap"); swapped.delete(i); }
    });
  });
  document.querySelectorAll(".verdict button").forEach((b) => b.addEventListener("click", () => {
    const tr = b.closest("tr"), i = tr.dataset.i;
    tr.querySelectorAll("button").forEach((x) => x.classList.remove("sel"));
    b.classList.add("sel");
    let v = b.dataset.v;
    if (v !== "~" && swapped.has(i)) v = v === "A" ? "B" : "A"; // un-shuffle blind votes
    votes[i] = v;
    const c = { A: 0, B: 0, "~": 0 };
    Object.values(votes).forEach((x) => c[x]++);
    document.getElementById("summary").textContent =
      \`old \${c.A} · tie \${c["~"]} · new \${c.B}  (\${Object.keys(votes).length}/${pairs.length})\`;
  }));
</script>`;
await writeFile(out, html);
console.log(`wrote ${out}  (${pairs.length} pairs, ${onlyB.length} new-only)`);
