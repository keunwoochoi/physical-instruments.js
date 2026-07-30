# Writing samples — the owner's actual voice, from primary sources

**Kind:** research / capture. Fetched 2026-07-28 at the owner's instruction ("study how i write").
This file exists because `author-and-voice.md` said *"no public writing samples fetched"* — it now has
them. That file owns the **target** voice for the report; this file owns the **evidence** it rests on.

## Corpus fetched

| Source | What it is | Register |
|---|---|---|
| [`keunwoochoi/beetbox-eval` README](https://github.com/keunwoochoi/beetbox-eval) | self-published tech report on a 17-model coding-agent eval, July 2026 | **the closest analogue to this report** |
| [Blog](https://keunwoochoi.github.io/blog.html) — 22 posts, 2023-10 → 2026-03 | build logs (undr.live ×3), paper readings, career posts ("Roffice Hour" ×7) | personal essay |
| [Solar Open 2 Tech Report](https://arxiv.org/abs/2607.20062) (250B-A15B, 50 authors) | corporate/consortium LLM report; he is a listed author, not the voice | formal lab report |
| [Solar Open 1 Tech Report](https://arxiv.org/pdf/2601.07022) (102B-A12B) | its predecessor | formal lab report |

**Weighting.** The blog and beetbox-eval are *his* voice. The Solar reports are a fifty-author
house style he contributed to — they are a **rigor and structure** reference, not a voice reference.
Do not mistake the second for the first.

> ⚠️ **Trap, hit during this fetch.** `WebFetch` returned a summarizer's paraphrase of the beetbox
> README that had silently rewritten first person into third — *"The creator observed a brief screen
> recording…"* for what he actually wrote: *"I saw a short screen recording of Beetbox… Well… If they
> could prompt, I can prompt it too."* Every stylistic marker was scrubbed. **Fetch raw markdown
> (`raw.githubusercontent.com`, `blog/*.md`) for any style work.** A summarizer is the one tool that
> destroys exactly the signal you are measuring.

---

## 0. Anti-slop list — owner-confirmed, 2026-07-28

**Standing owner directive (later the same day): "zero marketing bullshit, zero fake
impressiveness."** This is stronger than the voice analysis below and wins any conflict with it.
Concretely: no bolded taglines, no "Play it →" arrows, no winks (the feather-favicon aside), no
punchline paragraph closers, no styled sentiment beats ("I liked it"), no invented color (v2 of
the report added "at 11pm" to a debugging story — fabricated; this is how fake impressiveness
sneaks in as "voice"). State the fact, keep the number, stop.

The first draft of the report followed this file's analysis and the owner still called it
*"typical AI slop."* The tics that survived analysis and had to be caught by ear, so future
sessions catch them by list:

- **"not X — it is Y" reversals.** ("It is not a compression trick; it is what happens when…")
  The single most recognizable pattern. He states the fact once, without the foil.
- **Rule-of-three lists.** ("a CDN, a license, and a loading spinner.") He lists two, or four,
  or rambles — the polished tricolon is the tell.
- **Bolded aphorisms and chiasmus.** ("**Refusing evidence is a skill.**" / "cheap for a human,
  expensive for an agent.") His lessons are one flat line *after* the story: "Good lesson: …"
- **Performative honesty.** ("Honestly:", "I am not going to dress that up", "the single worst
  thing about the project.") He just states the limitation: "I didn't actually measure the token
  cost." No ceremony around candor.
- **Dramatic reveal framing.** ("Here is the part that changes how you should read §3.")
- **Cool-understatement beats.** ("I did not expect that to hold.") His beats are warm
  interjections — "I panicked." "Ugh." "I liked it."
- **Mic-drop paragraph closers on every paragraph.** One per section at most; most paragraphs
  should just end.
- **Third person about himself.** The build-session quotes are things *he typed*; the report must
  say "my messages," never "the owner."
- **Relentless punchy rhythm.** His sentences run long and loose with trailing "which…" clauses;
  a draft where every sentence is short and confident reads as AI regardless of content.

## 1. The register is *not* the register we planned

`style-and-structure.md` proposed "v03 rigor + Olah accessibility." The rigor half is right. But the
sampled voice is **much lighter, faster, more first-person, and funnier** than the v03 report, and it
is not Distill either — Distill is patient and pedagogical; he is impatient and conversational.

The nearest single sentence in the corpus to what this report should sound like:

> *"Then I realized the responsibility as a human… to judge them! \*cough cough taste is the moat cough
> cough\*, so I did it. Perhaps that simple eval is my biggest contribution to this webpage."*

That is a real methodological claim (human judgment is the contribution), a joke, and a hedge, in
three clauses. The whole voice is in there.

## 2. Structural signature

- **Headings are questions or blunt labels**, never noun-phrase academese: *"Why this website?"*,
  *"Why did I leave?"*, *"What am I doing?"*, *"Ok, so what?"*, *"Basically, Instagram + Gemini"*,
  *"Cheapest possible deployment"*, *"Descope and focus"*, *"It became a personal vibe check"*.
- **Numbered sections only in the technical mode** (beetbox: §1–7; Voxtral reading: §1–5). Personal
  posts use bare `##`.
- **Paragraphs are short — often one sentence.** Single-line paragraphs are used as beats:
  *"I panicked."* · *"ChatGPT."* · *"Yes. / Yes…"* · *"That's it.. ish."*
- **Limitations get their own top-level section, and it is titled in the first person**: beetbox §5 is
  literally *"Other aspects I didn't evaluate"* — four subsections of things he chose not to measure,
  placed *before* the discussion, not buried after it. This is the strongest structural inheritance
  available to us and it is already in our outline; keep it, and keep the first-person title.
- **The close is advice or a shrug, never a summary.** beetbox: *"So I'd suggest - try these models!
  Make something that you actually care about (and judge them!)"* Voxtral: *"# Conclusion / Yay ~ ~"*.
  undr.live: *"But I'm afraid that'd require another sabbatical of mine.."*
- **Sign-off convention:** `---`, then the date on its own line (`2026.03.27.`), then `Keunwoo`.
- **ASCII box-art pipeline diagrams**, hand-drawn, are a recurring signature figure (both undr.live
  posts). Not mermaid, not an image — monospace boxes and arrows.

## 3. Sentence-level markers

| Marker | Evidence |
|---|---|
| Double-dot ellipsis `..` | *"under $100/month"* ← *"So I still contribute to Google's revenue about.. under $100/month"* · *"That's it.. ish"* · *"But.."* · *"I.. can't work on this for too long"* |
| Audible interjections | *"Um, I wasn't impressed."* · *"Hm, yes perhaps.."* · *"Oh."* · *"Wow!"* · *"Boom!"* · *"Ugh."* · *"uh oh."* · *"whoa, too bad."* |
| Self-interruption mid-sentence | *"This automation was definitely worth the effort -- of setting up the LLC in the New Mexico, USA."* |
| Rhetorical question stacks answered immediately | *"Why would they? Why would they start doing it when I got nothing? What if they don't continue? No no no, that's too risky."* |
| Direct address / imperative to the reader | *"As you can tell if you clicked it, which you should"* · *"you'd rather check out the paper"* |
| Deflating self-aware aside after hard work | *"Wow! I spent a lot on this, as if people care about audio… haha!"* |
| Emphatic repetition for feeling | *"damn, DAAAAAMMNN."* · *"it was so annoying"* · *"Too good. Too revolutionary."* |
| Bolded lead-in on list items | `**Gemini** (API) handles…` / `**Electronics / Korea.**` — a bolded label then prose, not a colon-list |
| Typos left in | *"intentioanally"*, *"wen through"*, *"manitenance"*, *"an beetbox app"*. **Do not imitate this** — but read it as license to stop over-polishing. The prose is spoken, not buffed. |

## 4. How he handles evidence — this is the part to inherit wholesale

**Numbers are exact, small, and often about money or count, not just performance.**
*"I paid \$2.57 for the URL. From year 2, it will be \$26.26/year."* · *"~\$6/mo, which is under free
usage limit"* · *"~600KB"* · *"102 musicians, 369 musician-venue connections, 137 musician-musician
connections"* · *"44 of 369 connections currently have that level of confidence."*
He states the denominator. Always.

**He labels the confidence of his own claims, inline, without ceremony.**
- *"A K=1 trial is noisy, but I found the task useful for me."* (beetbox — the entire eval's caveat, in
  one clause, in the third paragraph)
- *"The prices are the official prices and I didn't actually measure the token cost of each version."*
- *"The classifier promotion threshold is Qwen score ≥ 7, set empirically. The golden set eval will
  validate it."* ← says the number **and** that it isn't validated yet
- *"Now that I think about it, I am not sure if that was fair."* ← flags his own protocol violation
  (he gave one model a retry hint) at the point it occurs, not in a limitations section

**He speculates openly and marks it as speculation, then says what would settle it.**
From the Voxtral reading, on a result he distrusts:
> *"I actually suspect something was wrong with the 'FLEURS fr' run… The loss curve is too noisy in
> general… the two curves seem too uncorrelated to me, which makes me suspect whether the training data
> loader was really reproducible **(To be honest, I also don't always do that.)**"*

The parenthetical is the move: he indicts himself in the same breath as the authors. And he ends the
thread with *"I still don't have any great conclusion from this, but it's nice to calculate the exact
frame size"* — an unresolved analysis, published unresolved.

**He shows the arithmetic in prose.** *"That's 2.5 to 3.3 words, or just the middle: 2.9 words/second.
Applying the general rule of thumb of a single word = 1.3 tokens, it translates to 3.77 tokens/second."*
Then uses it to challenge the paper's hypothesis. Derivation is the argument, run inline.

**Negative results get one flat sentence and no eulogy.** *"I turned it off."* · *"So I shut it down."*
· *"Um, I wasn't impressed."* · Kapre 0.4: *"**Changes**: all the broken unit tests were fixed; the code
is modernized. This is it.. yes, this is it. Nothing much different on its functionalities."*

**Judgment is stated, then immediately bounded.** The beetbox discussion is the model for our
"does it sound good?" section:
> *"GPT was more religious about following the input… Fable took more freedom. Its gradients are
> smoother, more complicated, and honestly very cool, but less literal… Which behavior is better
> depends on what the user wants… Sometimes it may actually be right. Other times, that is precisely
> the problem."*

Verdict → mechanism → the condition under which the verdict flips. Never a bare ranking.

**Rubrics are defined before they are applied.** beetbox §4 defines green/yellow/red for each of three
axes in three sentences each, then gives a *concrete failure instance* (*"the recording shows `C+` and
`A+`… Some agents seem to have read these as `C#` and `A#`"*). And he separates axes deliberately, and
explains why: *"I intentioanally distinguished this from Audio. As an example, a lead can play the
wrong pitch (low musicality) with a perfectly functional synthesizer (high audio score)."*

**He says who judged, and that it was him, by hand.** *"three manually judged columns"*. No pretense
of an automated oracle.

## 5. What he does with the "I built this with agents" thread

He has written this exact genre four times (seoulunderground, undr.live ×2, beetbox, plus Kapre 0.4
tagged *"this vibe-coded update"*). The established moves:

- **The build is framed as a personal need, not a demo.** *"I am the user, I know the frustration, I
  can judge the solution. This makes the whole process uncomparably faster and more accurate, as well
  as more fun."* — this is also his stated **thesis about why taste-driven building works**, and it is
  the same argument our Act II has to make.
- **The lesson is one bolded flat line, placed after the story, never before it.** *"Good lesson: Fast
  and accurate decisions only happen when you know the domain. When you're bootstrapping, do what you
  deeply care about."*
- **He already has a public position on `CLAUDE.md`/PRINCIPLES that our repo is an instance of**, and
  he states it in causal-LM terms rather than in agent-mysticism terms:
  > *"no matter how surprising LLMs are, they're causal language models. They take `x` … and produce
  > `y` … In setting up `.claude` files, what you're doing is constructing the input `x` for the LLM so
  > that the output `y` is what you actually want. The problem is that sometimes, most of what shapes a
  > good output is *unsaid* — things so obvious to you that you wouldn't think to write them down…
  > When it's just you and an LLM, you're still a company."*

  Our constitution + 7 personas + authority gates are the strongest existing evidence for that claim.
  **Cite the position; don't re-derive it.**
- **He reports on eval discipline as a temporal decision, not a virtue.** *"I still think it was correct
  to not have an eval at day 1. Eval requires a well-defined expectation, which requires a clear product
  spec, which you can only mature through actually building and using the product."* → Directly relevant
  to our "eval before trust" story and the loop-v3 rebuild: he will not accept "we should have had evals
  earlier" as the moral. The moral is *when* the eval became affordable.

## 6. What the Solar reports contribute (structure only)

Formal, plural "we", zero hedging tics, no jokes — **not our voice**. Inherit three things:

1. **The abstract is a mechanism chain, not a claim list.** Every sentence is *goal → the specific
   mechanism → the number*: *"To hold entire agent trajectories in a single context, Solar Open 2
   reaches a 1M-token window through a hybrid attention stack that interleaves one softmax layer among
   every three linear-attention layers…"* Our abstract should read like this: *to fit in 82 KB, we X;
   to keep the audio thread clean, we Y.*
2. **The conclusion names its own remaining gaps, specifically and without apology**, then generalizes:
   *"The remaining gaps are localized and point to the next steps: repository- and terminal-level
   software engineering… and fine-grained numerical precision in officework deliverables. Both call for
   stronger verification and self-checking behaviors in agent training."*
3. **Comparators carry a cost ratio, not just a win.** *"competitive with DeepSeek-V4-Pro (1.6T) at less
   than a sixth of its size."* Our whole artifact thread is a size argument — every quality claim must
   be stated against the KB it cost, in exactly this shape.

## 7. Direct consequences for the report — decisions this evidence forces

- **Pronoun: "I".** `style-and-structure.md` left this open. The corpus is unanimous — every personal
  and technical piece he publishes is first person, including the eval report that is structurally
  closest to ours. "We" would read as the Solar house style, which is a different, more anonymous
  author. *Recommendation, for the owner to confirm.*
- **Keep the raw quotes raw.** The other open question. He publishes his own frustration verbatim
  (*"Ugh."*, *"Um, I wasn't impressed."*, *"Ah crap"*, *"damn, DAAAAAMMNN"*) and prints his own errors
  in the same paragraph as his results. Lightly cleaning the agent-session quotes would be the one
  choice inconsistent with everything in this corpus. *Recommendation: keep them.*
- **Length: shorter than Distill.** His longest post (~155 lines of markdown) is a full build log with
  three diagrams. beetbox is 168 lines and covers 17 models, a rubric, and a discussion. The plan's
  "Distill-scale long-read" is out of character; the interactive layer should carry the depth, and the
  prose should stay fast.
- **Rename the limitations section** to first person, e.g. *"What I didn't evaluate"* / *"What I
  couldn't verify"*, and place it **before** the discussion, per beetbox.
- **Add an ASCII pipeline diagram** for the signal chain (MIDI → scheduler → worklet → WASM voice bank
  → mix), in box art. It is his signature figure and costs nothing.
- **The jokes are load-bearing, not decoration.** `style-and-structure.md` says "self-aware asides,
  sparingly — one or two land; more is a tic." The corpus disagrees: he uses them roughly once per
  section, and they consistently arrive *right after* the densest technical passage as a release
  valve. Budget accordingly.
