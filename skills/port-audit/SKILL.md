---
name: port-audit
description: Run before and during any port of third-party DSP code. License ledger check + legacy-flaws checklist. Usage - port-audit <source> <files>
---

# port-audit

## License gate (hard)
1. Confirm the source is on the approved list in `agentic-docs/licensing.md`. Not listed → stop, verify the license from the actual repo LICENSE file, add it to the ledger via PR first.
2. Copyleft (GPL/LGPL/AGPL) → **do not open the source.** Papers-only (see licensing.md). If copyleft code enters context, log it in the incident log and hand off to a fresh context.
3. Faust functions → verify the per-function license header before use.
4. Every ported file: ledger row + header comment (origin file, license, date).

## Legacy-flaws checklist (fix at port time, never copy through)
- [ ] Sample-rate hardcoding (Mutable 48k tables, STK global static SR) → per-instance SR, coefficients recomputed on change
- [ ] Global mutable state → per-instance
- [ ] On-disk asset loads (.raw excitations) → embed as const data or generate at init
- [ ] double-by-default → f32 unless numerically required (document exceptions)
- [ ] Mono-only → stereo-ready bus interface
- [ ] Per-sample branchy loops → block-based, SoA/SIMD-batchable shape
- [ ] No parameter smoothing → use engine smoothing primitives
- [ ] No denormal handling → flush-to-zero on all recursive state
- [ ] Excitation baked into instrument → split exciter ↔ resonator ↔ body
- [ ] Allocation in audio path (incl. constructors called from note-on) → voice pool
