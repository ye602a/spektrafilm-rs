#!/usr/bin/env python3
"""Fail if the versioned FPW calibration/shader/reference bytes drift.

This uses only the Python standard library and does not download anything.
The source commit and extraction rules are in
docs/maintenance/CALIBRATED_OUTPUT_V1.md and data/negative_scan_ap0_v1/PROVENANCE.md.
An intentional algorithm/table change must introduce another route version.
"""

from hashlib import sha256
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "data/negative_scan_ap0_v1/pro400h.json":
        "6728ba04757b352d70756367efd1f2465f2b15ed4d1ba6882a0bef8c589606b3",
    "data/negative_scan_ap0_v1/xtra.json":
        "85db3fc1733979c95a53f622f4ba0cb137fab7eecd2b936529634bad23a69ad8",
    "data/negative_scan_ap0_v1/heldout.json":
        "fd8d58649ce26b31f6c919f6ca4d7d668e2d557aca3ce4e87542a0e2c37b34ce",
    "crates/spektrafilm-shaders/wgsl/physical_output_v1.wgsl":
        "d5220ed250dc27a2812065415f92c9fd0edb4efa6144f418aa0cd4139e7e160d",
    "crates/spektrafilm-core/tests/fixtures/fpw_physical_output_v1.wgsl":
        "d5220ed250dc27a2812065415f92c9fd0edb4efa6144f418aa0cd4139e7e160d",
    "crates/spektrafilm-core/tests/fixtures/fpw_negative_scan_v1.rs":
        "f0bc02d01bf705a6a2595e1137fced2c8ad84087d8a1e17641054a00401529e6",
    "crates/spektrafilm-core/tests/fixtures/fpw_finish_v1.rs":
        "96dad0de5f534b2f607bc894cfd1a6708e8094d67e972eda10d92fb9125eacd1",
}


def main() -> int:
    failures = []
    for relative, expected in EXPECTED.items():
        path = ROOT / relative
        if not path.is_file():
            failures.append(f"missing frozen source: {relative}")
            continue
        actual = sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            failures.append(f"frozen source drift: {relative}\n  expected {expected}\n  actual   {actual}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"Verified {len(EXPECTED)} frozen negative_scan_ap0_v1 source/fixture hashes.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
