#!/usr/bin/env python3
"""Compare complete synthetic GPU migration probes from two source revisions.

Usage: python tools/validation/compare_migration_probe.py BASELINE CANDIDATE
Input geometry, source coordinates, film/paper configuration, parameters and
input pixels must match before output pixels can form a valid comparison.
"""

import hashlib
import json
import math
from pathlib import Path
import struct
import sys


def input_manifest(directory: Path, output_names: set[str]) -> dict:
    manifest = json.loads((directory / "manifest.json").read_text())
    required = {"fixture", "reference_revision", "format", "film", "paper", "cases"}
    if not isinstance(manifest, dict) or required - manifest.keys():
        raise ValueError(f"{directory}: missing required input manifest fields")
    cases = manifest["cases"]
    if not isinstance(cases, list) or not cases:
        raise ValueError(f"{directory}: the manifest must contain input cases")
    case_inputs = {}
    required_case = {
        "name", "width", "height", "gpu_multilayer", "scan_film", "source_reference", "bytes"
    }
    for case in cases:
        if not isinstance(case, dict) or required_case - case.keys():
            raise ValueError(f"{directory}: incomplete input case metadata")
        name = case["name"]
        if not isinstance(name, str) or name in case_inputs:
            raise ValueError(f"{directory}: invalid or duplicate case name {name!r}")
        width, height = case["width"], case["height"]
        if type(width) is not int or type(height) is not int or width <= 0 or height <= 0:
            raise ValueError(f"{directory}/{name}: invalid image geometry")
        if case["bytes"] != width * height * 3 * 4:
            raise ValueError(f"{directory}/{name}: byte count does not match RGB f32 geometry")
        # These three fields summarize the rendered result. All other fields
        # describe the fixture or its configuration and must match exactly.
        case_inputs[name] = {
            key: value for key, value in case.items() if key not in {"min", "max", "mean"}
        }
    if {f"{name}.f32" for name in case_inputs} != output_names:
        raise ValueError(f"{directory}: manifest cases and output files do not match")
    return {**manifest, "cases": case_inputs}


def compare(baseline: Path, candidate: Path) -> dict:
    expected = {path.name for path in baseline.glob("*.f32")}
    actual = {path.name for path in candidate.glob("*.f32")}
    if not expected or expected != actual:
        raise ValueError(
            f"Output cases differ: missing={expected - actual}, extra={actual - expected}"
        )
    old_manifest = input_manifest(baseline, expected)
    new_manifest = input_manifest(candidate, actual)
    if old_manifest != new_manifest:
        raise ValueError(
            "Input manifest changed (geometry, source reference, film/paper or other "
            "configuration); output comparison is invalid"
        )

    results = []
    passed = True
    for name in sorted(expected):
        for folder, suffix in [("inputs", ".f32"), ("params", ".json")]:
            metadata_name = Path(name).stem + suffix
            if (baseline / folder / metadata_name).read_bytes() != (
                candidate / folder / metadata_name
            ).read_bytes():
                raise ValueError(
                    f"Input/parameters changed for {folder}/{metadata_name}; invalid comparison"
                )
        old = (baseline / name).read_bytes()
        new = (candidate / name).read_bytes()
        expected_bytes = old_manifest["cases"][Path(name).stem]["bytes"]
        if len(old) != expected_bytes:
            raise ValueError(f"{name}: baseline output does not match declared geometry")
        same = old == new
        passed &= same
        row = {
            "case": Path(name).stem,
            "byte_identical": same,
            "baseline_bytes": len(old),
            "candidate_bytes": len(new),
            "baseline_sha256": hashlib.sha256(old).hexdigest(),
            "candidate_sha256": hashlib.sha256(new).hexdigest(),
        }
        if len(old) == len(new) and len(old) % 4 == 0:
            old_values = [value[0] for value in struct.iter_unpack("<f", old)]
            new_values = [value[0] for value in struct.iter_unpack("<f", new)]
            if not all(map(math.isfinite, old_values)):
                raise ValueError(f"{name}: baseline has nonfinite pixels")
            finite = all(map(math.isfinite, new_values))
            passed &= finite
            row["changed_scalar_bits"] = sum(
                old[index:index + 4] != new[index:index + 4]
                for index in range(0, len(old), 4)
            )
            row["candidate_all_finite"] = finite
            row["max_absolute_error"] = (
                max(abs(a - b) for a, b in zip(old_values, new_values)) if finite else None
            )
        results.append(row)
    return {
        "baseline_directory": str(baseline),
        "candidate_directory": str(candidate),
        "input_configuration_identical": True,
        "all_byte_identical": passed,
        "case_count": len(results),
        "results": results,
    }


def main() -> int:
    if len(sys.argv) != 3:
        raise ValueError("Usage: compare_migration_probe.py BASELINE CANDIDATE")
    report = compare(Path(sys.argv[1]), Path(sys.argv[2]))
    print(json.dumps(report, indent=2, allow_nan=False))
    return 0 if report["all_byte_identical"] else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError) as error:
        raise SystemExit(f"Invalid migration probe comparison: {error}") from error
