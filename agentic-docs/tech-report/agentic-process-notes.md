# Agentic-process notes — raw material for thread #2

**Provenance.** Captured from the build session of 2026-07-22 → 07-23 (this session), during
which the interactive demo, mobile support, deploy, analytics, CI repair, and audio fixes were
built by the owner directing a Claude Code agent. This is the primary source for the process
thread. Quotes marked *(owner, verbatim)* are the owner's actual typed words — per `CLAUDE.md`,
authorship is not evidence of voice, so the owner's own words are flagged as such. Nothing here
is sanitized; the friction is the contribution.

## The build arc (roughly chronological)
1. **Drum note-view** → a dedicated falling-note drum panel beside the piano roll.
2. **Interactive rework** → retire the transport button row; the whole "video" area is
   play/pause (YouTube-style); ←/→ seek 10 s; a floating legend; a distinct drum view
   (falling *diamonds* + component *pads*, not piano keys); Windup as the default demo;
   **click keys/pads to play them**; instrument dropdown; drum-kit dropdown; **live Web MIDI in**;
   bigger fonts; pause-holds-position.
3. **Ship** → commit to `main`, push, deploy to **GitHub Pages** (live), favicon (a feather —
   a wink at the ~82 KB), header copy stating the size/instrument-count.
4. **Analytics** → GA4 wired into the demo.
5. **CI repair** → "fix this" cascaded into a chain of pre-existing environment-drift failures.
6. **Mobile** → responsive geometry + layout; iOS/Android audio-unlock; background cleanup.
7. **Genre drum kits** → per-song kit preselected by genre.
8. **This planning conversation** → the tech report.

## Friction / failure ledger (the honest bits — keep these)
- **The worktree mirage.** Owner: *"i don't see the change.. where?"* … *"the changes are not
  there."* Root cause: the owner's dev server on `:8174` was rooted in a **different git
  worktree** (`vst.js-spike`, branch `spike/piano-polarizations`), while edits landed in the
  `main` worktree. Same `.git`, different checkout. Diagnosed by reading the server process's
  cwd. *The human hit a failure mode (git worktrees) they neither created nor needed to know
  about.*
- **Wrong-session bleed.** Mid-flow the owner issued several requests ("move show reference,"
  "Beetbox reference capture," eval-column tooltips) that turned out to belong to a **different
  project** (`beetbox-eval`) in another session — *"oh sorry wrong session."* The agent had
  already started researching them. *Context/session boundaries are a real UX seam.*
- **The "no sound" saga (the big one).** The demo animated and the timer ran, but there was no
  audio. The agent's headless tests had reported "playing" — but they launched Chromium with
  `--autoplay-policy=no-user-gesture-required`, which **auto-resumes the AudioContext**, so
  "playing" (glyph hidden, time advancing) *never proved audio*. **A false "verified."** Then:
  a mute-switch red herring; a synchronous-iOS-resume fix; then owner: *"it is even more broken
  now. the playback stops in one second."* Then the sharp clue — owner: *"the key interactively
  works. but not the playback. oh when i manually choose another song, then it works. but when
  it opens in the first time, first song preselected one, it doesn't."* Instrumenting `play()`
  caught it: first play scheduled against a **cold AudioContext clock** (`ct=0.009` vs. a warm
  `ct=4.133` after switching), so timestamped notes landed in the past → silence; untimed key
  presses worked. Fix: wait for the clock to tick before scheduling. **The agent could not
  reproduce the iOS failure headlessly and said so** — ground truth stayed on the owner's device.
- **CI archaeology.** "fix this" (a red `typescript` job) unpacked into a *cascade*: missing
  `libsndfile` → a **pinned libsndfile 1.0.31** (Ubuntu 22.04, because `ubuntu-latest` moved to
  24.04/1.2.2) → a `dtolnay/rust-toolchain` conflict on 22.04 → a **stale test** (GM 41→viola,
  57→trombone) → **WASM byte-identity** drift → **transport-baseline drift from 37 intended DSP
  commits**. The agent fixed the mechanical/clear ones and **stopped at the two owner-decision
  boundaries** (rebaseline golden audio; WASM reproducibility pinning) rather than guess — both
  gated by the repo's own scripts ("changes require separate review").
- **Infra papercuts.** The `gh` CLI's active account kept **reverting** from `keunwoochoi` to
  `keunwoo-ortet` (no admin on the repo) → 403s. **Actions was disabled** on the repo. **`dist/`
  is gitignored** so Pages had to build it in CI. The **README was stale** (74 KB / 15
  instruments; code said ~82 KB / 29 — "code owns behavior," so the code won).
- **Third-party tooling.** Owner on the analytics setup: *"ok cloudflare is fucked up."* …
  *"ah fuck wait."* → pivoted (GoatCounter offered) → back to Cloudflare → landed on GA4. A
  reminder that the friction isn't only in the code.
- **Taste corrections.** Owner: *"what is 'song's own' the fuck. show one of the drum kit being
  used... checked.. gosh.."* (an abstraction the human found opaque). Owner: *"everywhere, the
  font is extremely extremely small. they should be 2 level bigger all."* (a global taste call
  the agent had gotten wrong at the default.)

## Expectations — correctly anticipated vs. not
- **Correctly anticipated by the owner:** the iPhone hardware **mute switch** as a sound
  culprit; that the demo *should* be mobile-friendly; that **genre should drive the kit**;
  most taste-level UX calls ("keys should sound piano," "drums should look distinct from the
  piano roll," title should explain the demo).
- **Did NOT expect / did not need to know:** git worktrees serving a stale copy; libsndfile
  version pinning; the Ubuntu-runner drift; Rust→WASM byte-reproducibility; the headless
  autoplay-policy testing gap; the iOS cold-clock scheduling bug. *These are exactly the layer
  the agent was expected to own — and the report's honest point is that the human shouldn't
  have to hold them.*

## Division of labor / knowledge
- **Human (owner) owned:** direction, taste, the ear ("sound piano!"), UX intent, genre/kit
  choices, ship/defer decisions, catching false results on real devices, priorities.
- **Agent owned:** implementation, DSP/engine internals, CI/deploy plumbing, diagnosis and
  instrumentation, verification (**with stated limits** — e.g. couldn't verify real iOS),
  stopping at owner-decision boundaries.
- **The interesting seam:** the agent's most useful move was often *not* writing code but
  *diagnosing* (worktree cwd; analyser-measured audio level; `play()` instrumentation) and
  *refusing to guess* at owner-judgment calls (rebaseline; WASM pinning).

## Where the agent was wrong (own it — this is the most valuable section)
- **Shipped a false "verified" on audio** because the test harness masked the bug it was meant
  to catch. Corrected only when the owner heard silence on a real device.
- **One "fix" made it worse** ("stops in one second") before the real cause was found.
- Repeatedly **over-trusted headless green** as proof of a user-facing behavior it couldn't
  actually observe (audio). The lesson the report should draw: *an agent's confidence is only
  as good as what its tools can observe; say what you cannot verify.*

## Satisfaction / dissatisfaction beats
- Dissatisfaction: the quotes above (worktree, "song's own," fonts, "even more broken,"
  cloudflare).
- Satisfaction: *"cool!"*, *"great"*, *"ok good"*, *"ok ship it"*, *"oh shipped good"*; the
  feather favicon; the genre kits; the mobile pass landing.

## Episode: the loudness question (2026-07-23) — domain expertise as the catch
During report planning, the owner asked whether the engine actually does perceptual loudness
normalization across instruments — "which I once told you to use pyloudnorm … so the volume is
normalized across different instruments and different notes." This is a **domain-expert
verification question**: he knows LUFS/pyloudnorm from music-AI work, so he knew *what to check*.

Investigating exposed a real gap the agent had left standing: `makeup_gain` in `kernels.rs`
LUFS-matched most families but left **9 instruments (bowed strings, brass, organ) "provisional"
at unity**, and `measure-loudness.mjs` **only covered 15 of 29 instruments**, so those 9 couldn't
even be re-measured. The agent extended the harness to all 29, re-baked from `pyloudnorm` BS.1770
LUFS, and **26/29 now land at −22.5 LUFS ±~1 dB** (cello was ~6 dB hot, contrabass ~7 dB low).
The **brass (trumpet/sax/french-horn) measured −44/−40/−58 LUFS** — a 20–35 dB *source-level*
deficit; their honest makeup would be ×11.7/×7.3/×59, so they were clamped to 3.0× and flagged.

*Thread-#2 point:* the human didn't need to know how the makeup table worked, but his expertise
let him ask the one verification question that surfaced an incomplete job the agent had marked
"provisional" and moved on from. Expertise here bought **the right question**, not the fix.
(For the report's *evaluation* section, this is the honest eval spine: a measured before/after
loudness table, plus the brass caveat as a stated limitation — no human listening study.)

## Candidate thesis points for the writeup (thread #2)
- Taste stays human; plumbing delegates — but the *boundary* is negotiated live, not fixed.
- The failure modes that bite are rarely the domain (DSP); they're the seams (worktrees, CI
  env drift, autoplay policy, iOS clocks, third-party dashboards).
- An agent that **states what it cannot verify** and **stops at judgment boundaries** is more
  trustworthy than one that's always green.
- The commit log / this KB *is* the research record — the process is legible because it was
  written down as it happened.
