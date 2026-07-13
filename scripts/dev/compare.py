#!/usr/bin/env python3
"""CLI for the versioned instruments.js reference-comparison metric kernel."""

import argparse
import json

import loop_metrics


def parse_args(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("render")
    p.add_argument("reference")
    p.add_argument("--profile", choices=sorted(loop_metrics.PROFILES), default="default")
    p.add_argument("--flat-weighting", action="store_true")
    p.add_argument("--expected-onset-s", type=float)
    p.add_argument("--note-off-s", type=float)
    p.add_argument("--max-post-note-off-db", type=float)
    p.add_argument("--expected-f0", type=float)
    p.add_argument("--partial-model-json", help="JSON object declaring proximity_harmonic, stiff_string, or modal_ratios pairing")
    p.add_argument("--json", action="store_true", help="retained for compatibility; JSON is always emitted")
    return p.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    partial_model = json.loads(args.partial_model_json) if args.partial_model_json else None
    if partial_model is not None and not isinstance(partial_model, dict):
        raise ValueError("--partial-model-json must decode to an object")
    report = loop_metrics.compare_files(
        args.render,
        args.reference,
        profile=args.profile,
        flat=args.flat_weighting,
        expected_onset_s=args.expected_onset_s,
        note_off_s=args.note_off_s,
        max_post_note_off_db=args.max_post_note_off_db,
        expected_f0=args.expected_f0,
        partial_model=partial_model,
    )
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
