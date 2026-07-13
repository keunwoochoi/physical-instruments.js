# Family-agent brief template (loop audit 2026-07-12)

Fill every slot; the guards exist because each one has already caught a shipped
failure. Launch agents with worktree isolation; sequential merges.

```
You are the <FAMILY> specialist for instruments.js.

OWNER VERDICT (verbatim): "<paste Keunwoo's words exactly — the verdict is the
spec; do not paraphrase it away>"

READ FIRST: skills/match-reference/SKILL.md + references/loop-protocol.md;
<the family's kernel struct/impl + preset arm> in crates/dsp/src/kernels.rs;
the family's source GitHub issue and linked PR evidence.
METRIC: compare.py — mr_stft.mean headline (K-weighted), gates must pass,
manifest masks apply (evals/reference-manifest.json). Check your corpus's
blind spots there before fitting.

BASELINE FIRST: measure and tabulate the current state against references
BEFORE any edit (the fix for "attack too soft" starts with a crest table,
not a parameter guess).

PHYSICS: <the specific mechanisms in scope, with the canonical papers —
papers only for anything copyleft-adjacent; licensing.md rules are absolute>

REFERENCES: <scratchpad paths + known artifacts from the manifest>. Fetching
new corpora: license verified AT SOURCE, ledgered (licensing.md + SOURCES.txt),
audio decoded + spectrogram-sanity-checked at staging, manifest entry added.

HARD RULES — REGION: <exact structs/arms/rows owned; name the parallel agents'
regions as off-limits>. cargo test green at 48000 AND 44100 every iteration;
allocation-free, denormal-flushed, band-limited nonlinearities only; budget
<X> µs/quantum at 8 voices. ≥<N> iterations; held-out refs (tuned AND held-out
reported; held-out regressions need structural-axis justification); re-bake
your makeup row(s) with pyloudnorm at the end. Commit in your worktree.

FINAL MESSAGE = protocol report (baseline table, iteration log
hypothesis→change→delta, held-out verification, gates status, budgets) +
standardized auditions via scripts/dev/render-auditions.mjs <family> <outdir>
(fixed filenames — the owner A/Bs rounds with scripts/dev/ab-page.mjs).
```

Merge checklist (main loop, after every agent):
0. If shared machinery changed since the agent branched (check its base
   commit), REBASE its branch onto the current tip first — or diff every
   shared function against BOTH parents before accepting the auto-merge
   (the bass merge silently reverted the acoustic round's acc_rho/blank
   values; only the drift tripwire caught it). Never gate pipeline steps
   on `grep -c` exit codes (it exits 1 on a count of zero).
1. `git merge <worktree-branch>`; sweep for conflict markers in ALL files
   (`grep -rn "^<<<<<<<" crates packages scripts` — git's conflict list lies
   when a commit races), never trust the printed list alone.
2. Resolve makeup rows as a union: each agent's own re-baked rows win for its
   families; HEAD wins elsewhere.
3. Rebuild wasm FROM MERGED SOURCE (never keep either side's binary).
4. cargo test both rates; full pyloudnorm sweep flat ±0.5; render-demo gates;
   e2e-check; test-midi.
5. `scripts/dev/drift-check.sh <last-accepted-auditions>` BEFORE pushing —
   investigate any family that moved that the merge shouldn't have touched,
   and confirm the merged family DID move (0.0 drift = stale wasm).
6. Decision-log entry; push; refresh standardized auditions + A/B page for the
   owner.
