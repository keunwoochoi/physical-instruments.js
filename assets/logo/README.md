# assets/logo — derived brand assets

Everything in this directory, plus `apps/playground/favicon-pixel.svg`, is
**generated**. Never edit any of it by hand; regenerate with:

```sh
scripts/dev/render-logo.sh   # needs librsvg (rsvg-convert) + ImageMagick
```

## Derivation chain

```
apps/playground/favicon.svg          the source mark (hand-drawn vector; the only input)
        │
        ▼  scripts/dev/gen-logo-mosaic.py
assets/logo/logo.svg                 square-tile mosaic (rounded tiles + grout)
apps/playground/favicon-pixel.svg    the same tile grid as hard pixel art (what the tab shows)
        │
        ▼  rsvg-convert (in render-logo.sh)
assets/logo/logo-{1024,512,256,128}.png
```

## Design intent (owner-approved, 2026-07-30)

- One visual system across the sets-of-instruments repos: a shared dark plate
  and one mark with one finish per repo — the amber feather here, `[-]` in
  steel blue for subtractive-synthesizers.js; future synth families follow a
  bracket-glyph scheme (e.g. `[+]` for additive).
- The **logo** is a coarse mosaic of rounded tiles with grout gaps, every tile
  snapped to a fixed two-tone amber palette so it reads as deliberate pixel
  art. The favicon's carved barbs and shaft are thinner than one tile, so the
  generator runs cleanup passes: close accidental holes to fixpoint, re-carve
  the shaft as one deliberate dark staircase along the favicon's rachis, drop
  orphan tiles.
- The **favicon** is *not* the mosaic shrunk — grout muddies the mark at tab
  sizes. It is the same tile grid rendered as full-bleed hard pixels, so the
  tab icon and the logo are the same artwork at different fidelities.

All tunables (grid size, fit scale, palette, pass thresholds) live as named
constants in `scripts/dev/gen-logo-mosaic.py` — that script owns those facts.
The full design journey, including abandoned routes and the measurements that
drove each decision, is in the `git log` of this directory, PR #94, and the
journey log (issue #51).
