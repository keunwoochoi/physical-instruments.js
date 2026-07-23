#!/usr/bin/env python3
"""Reference vs render, on every axis at once. The gap, in the reference's own units."""
import json, sys, math, statistics as st
ref = json.load(open(sys.argv[1])); our = json.load(open(sys.argv[2]))
name = sys.argv[3] if len(sys.argv) > 3 else "instrument"
db = lambda v: 20 * math.log10(max(v, 1e-6))

def vel(x):
    f = x["file"]
    return f.split("_v")[1][0]

def key(x):
    return (round(x["midi"]), vel(x))

R = {key(x): x for x in ref}
O = {key(x): x for x in our}
common = sorted(set(R) & set(O))
print(f"  ===== {name.upper()}: OURS vs REAL, {len(common)} matched notes =====\n")
print("   midi vel |  centroid ref/ours    |  attack ref/ours   | h2 ref/ours | h3 ref/ours")
for k in common:
    r, o = R[k], O[k]
    print(f"   {k[0]:4d}  v{k[1]}  | {r['centroid']:6.0f} /{o['centroid']:6.0f} Hz "
          f"{db(o['centroid']/r['centroid'])*0+ (o['centroid']-r['centroid']):+6.0f} | "
          f"{r['attack_ms']:4.0f} /{o['attack_ms']:4.0f} ms | "
          f"{db(r['h'][1]):5.1f} /{db(o['h'][1]):5.1f} | {db(r['h'][2]):5.1f} /{db(o['h'][2]):5.1f}")

print("\n   ---- AGGREGATE GAPS ----")
for v in ["1", "2", "3"]:
    ks = [k for k in common if k[1] == v]
    if not ks: continue
    rc = st.median([R[k]["centroid"] for k in ks]); oc = st.median([O[k]["centroid"] for k in ks])
    ra = st.median([R[k]["attack_ms"] for k in ks]); oa = st.median([O[k]["attack_ms"] for k in ks])
    print(f"   v{v}: centroid real {rc:5.0f} Hz  ours {oc:5.0f} Hz  ({oc/rc:.2f}x)   "
          f"attack real {ra:4.0f} ms  ours {oa:4.0f} ms")

lo = [k for k in common if k[1] == "1"]; hi = [k for k in common if k[1] == "3"]
if lo and hi:
    rdyn = st.median([R[k]["lufs"] for k in hi]) - st.median([R[k]["lufs"] for k in lo])
    odyn = st.median([O[k]["lufs"] for k in hi]) - st.median([O[k]["lufs"] for k in lo])
    rbr = st.median([R[k]["centroid"] for k in hi]) / st.median([R[k]["centroid"] for k in lo])
    obr = st.median([O[k]["centroid"] for k in hi]) / st.median([O[k]["centroid"] for k in lo])
    print(f"\n   DYNAMIC RANGE pp->ff:  real {rdyn:5.1f} dB   ours {odyn:5.1f} dB")
    print(f"   BRIGHTNESS pp->ff:     real {rbr:5.2f}x    ours {obr:5.2f}x")

print("\n   HARMONIC LADDER (median over all notes, dB rel h1)  <- the shape of the tone")
print("        h2     h3     h4     h5     h6     h7     h8     h9    h10")
for lbl, S in (("real", R), ("ours", O)):
    row = "".join(f"{st.median([db(S[k]['h'][h]) for k in common]):7.1f}" for h in range(1, 10))
    print(f"   {lbl}{row}")
row = "".join(f"{st.median([db(O[k]['h'][h]) for k in common]) - st.median([db(R[k]['h'][h]) for k in common]):+7.1f}" for h in range(1, 10))
print(f"   GAP {row}")
