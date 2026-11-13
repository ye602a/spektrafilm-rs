"""Compare the two synthetic physical-plane runs; no external Python packages."""
from pathlib import Path
import argparse
import hashlib
import json
import math
import struct


def compare_params(left, right, path=""):
    if type(left) is not type(right):
        return [(path, left, right)]
    rows = []
    if isinstance(left, dict):
        for key in sorted(left.keys() | right.keys()):
            if key not in left or key not in right:
                rows.append((path + "." + key, left.get(key), right.get(key)))
            else:
                rows.extend(compare_params(left[key], right[key], path + "." + key))
    elif isinstance(left, list):
        if len(left) != len(right):
            rows.append((path, left, right))
        else:
            for index, (a, b) in enumerate(zip(left, right)):
                rows.extend(compare_params(a, b, path + f"[{index}]"))
    elif left != right:
        rows.append((path, left, right))
    return rows


def compare(reference, candidate, candidate_commit):
    records = []
    for case in json.loads((reference / "cases.json").read_text()):
        name = case["name"]
        a = (reference / (name + ".f32")).read_bytes()
        b = (candidate / (name + ".f32")).read_bytes()
        assert len(a) == len(b), name
        source = (reference / (name + ".input.f32")).read_bytes()
        assert source == (candidate / (name + ".input.f32")).read_bytes(), name
        va = [x[0] for x in struct.iter_unpack("<f", a)]
        vb = [x[0] for x in struct.iter_unpack("<f", b)]
        assert all(math.isfinite(x) for x in va + vb), name
        assert len(set(va)) > 1 and len(set(vb)) > 1, name
        differences = [abs(x - y) for x, y in zip(va, vb)]
        pa = json.loads((reference / (name + ".params.json")).read_text())
        pb = json.loads((candidate / (name + ".params.json")).read_text())
        assert pb["film_render"]["grain"].pop("gpu_multilayer") is True, name
        records.append({
            **case,
            "fpw_sha256": hashlib.sha256(a).hexdigest(),
            "rs_sha256": hashlib.sha256(b).hexdigest(),
            "input_sha256": hashlib.sha256(source).hexdigest(),
            "bit_exact": a == b,
            "different_f32_words": sum(a[i:i + 4] != b[i:i + 4] for i in range(0, len(a), 4)),
            "max_abs": max(differences),
            "rmse": math.sqrt(sum(d * d for d in differences) / len(differences)),
            "reference_min": min(va),
            "reference_max": max(va),
            "reference_distinct_f32_values": len(set(va)),
            "parameter_differences_after_expected_multilayer_optin": compare_params(pa, pb),
        })
    return {
        "reference": "FPW ae3428acd98e39c48784a79dc8f3369ab1236cb1 exact numeric sources with original declared dependencies",
        "candidate": candidate_commit,
        "case_count": len(records),
        "total_channels": sum(x["channels"] for x in records),
        "all_bit_exact": all(x["bit_exact"] for x in records),
        "all_effective_params_equal": all(not x["parameter_differences_after_expected_multilayer_optin"] for x in records),
        "max_abs": max(x["max_abs"] for x in records),
        "cases": records,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("reference", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--candidate-commit", default="c0308a25e5d98f8db9864ceef191e2a9aeee4e43")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.reference, args.candidate, args.candidate_commit)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "cases"}))
    raise SystemExit(0 if result["all_bit_exact"] and result["all_effective_params_equal"] else 1)
