# Audience — DECIDED (2026-07-23)

**Decision.** Framing **D — Layered / dual-track** (accessible spine + progressive-disclosure
depth). Thread balance: **artifact as the teaser (Act I), agentic process as the second-half
payload (Act II)**, *braided* — see the two conditions below and the Act structure in
`style-and-structure.md`.

**Two conditions (agreed) so it still clears the AES/NIME/ISMIR bar:**
1. The Act-I teaser **carries the honest rigor spine** (sound eval + real numbers), presented
   accessibly — a teaser, *not* a trailer/marketing.
2. **Braid, don't sequence:** plant seed→payoff hooks in Act I, and let the Act-II process
   narrative *carry* the deep technical detail (the iOS-audio saga delivers the Web-Audio
   depth; the CI cascade delivers the WASM/reproducibility depth). No dry standalone "specs"
   section, no disconnected "memoir."

---

## Background (why this framing)

It drives everything after (style, depth, what we explain vs. assume, which reviewers gate).

## The core tension
The quality bar is **award-venue** (ISMIR/AES/NIME/ICML-workshop) — which is *expert-facing*
rigor — but the owner also wants it **"interesting, fun, exciting, educating a lot to some
people,"** with a Distill/Olah interactive layer — which is *generalist-facing* reach. Those
pull opposite ways on depth and vocabulary. **The Distill move is to resolve the tension by
layering**, not by picking one and alienating the other.

## Options
**A — Curious technical generalists (broad).** HN / dev-adjacent / AI-curious readers who
love a well-told build story. Music-AI/DSP experts served in optional depth. *Widest reach;
risk: experts find it thin, undercutting the award bar.*

**B — Practitioners (niche-deep).** Music-AI + web-audio + physical-modeling + creative-coding
people who might use or extend it. *Satisfies Juhan/Yotam/producer; risk: loses the lay reader
and most of the "fun" reach.*

**C — The agentic-AI audience (process-led).** "How we build software with LLMs now"; the
instrument is the case study, the collaboration is the thesis. *Rides the zeitgeist; risk:
the music/DSP contribution becomes decoration.*

**D — Layered / dual-track (recommended).** One readable spine everyone can follow, with
progressive-disclosure depth (expandable proofs, spectrograms, budgets, code) for experts —
Distill's native form. **Primary:** curious technical readers spanning music/AI/web who will
happily go one layer deep. **Must-not-fail secondary:** the domain experts (Juhan-type) on
rigor, and the lay reader on wonder. Both threads (artifact + process) co-lead.

## Recommendation
**D**, framed as: *write so a curious non-expert gets the whole story and finishes excited,
and so Juhan cannot fault a single number.* That is exactly the Distill contract, and it is
the only framing that honors both the award bar and the "fun/educational" goal at once.

Concretely, three reader tiers we design for:
- **Tier 1 (spine):** lay-curious → follows the narrative, hears the sounds, gets the payoff.
- **Tier 2 (one layer down):** technical generalist / vibe-coder / AI person → the "how",
  the interactive figures, the honest process ledger.
- **Tier 3 (deep):** CCRMA/DSP/web-audio/systems experts → the DSP, the eval, the budget
  numbers, reproducibility, the failure analyses.

## Open questions for the owner
1. Pick a primary framing: **A / B / C / D** (recommend **D**).
2. Of the two threads, is it **artifact-led with process braided**, **process-led with
   artifact as case study**, or **truly co-equal**? (affects the title and the opening.)
3. Any audience we're deliberately **not** writing for? (e.g. pure academics who'd want a PDF;
   pure lay readers who'd bounce off any code.)
4. Publication venue/home in mind (personal blog? arXiv-style? a Distill-like standalone?) —
   shapes how far we push interactivity in v1 vs. later.
