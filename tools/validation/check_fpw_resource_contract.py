#!/usr/bin/env python3
"""Verify the pinned FPW physical resource contents without FPW repository access.

The default checks RS's 55 shared engine resources and 3 calibrated-output JSON
artifacts. --data-dir checks only the 55 shared resources in an installed FPW
physical resource directory. The host's ICC and digital LUTs are outside scope.
Only recognized UTF-8 text resources permit LF/CRLF differences; numeric tokens,
formatting, final newlines and all binary LUT bytes otherwise remain exact.
"""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import sys


CONTRACT_ID = "fpw-bundled-physical-resources-v1"
TEXT_SUFFIXES = {".json", ".csv", ".txt", ".toml"}
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO_ROOT / "docs/maintenance/validation/fpw_resources_20261113.json"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def normalize(data: bytes, rule: str, path: str) -> bytes:
    if rule == "none":
        return data
    if rule != "utf8_crlf_to_lf" or PurePosixPath(path).suffix not in TEXT_SUFFIXES:
        raise ValueError(f"unsupported text normalization for {path}")
    data.decode("utf-8", errors="strict")
    normalized = data.replace(b"\r\n", b"\n")
    if b"\r" in normalized:
        raise ValueError("bare CR is outside the pinned LF/CRLF text contract")
    return normalized


def set_identity(entries: list[dict]) -> str:
    pairs = sorted((entry["path"], entry["canonical_sha256"]) for entry in entries)
    return sha256(json.dumps(pairs, ensure_ascii=True, separators=(",", ":")).encode("ascii"))


def load_manifest(path: Path) -> dict:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1 or manifest.get("contract_id") != CONTRACT_ID:
        raise ValueError("unsupported resource manifest version or contract")
    for entries_key, count_key, identity_key in [
        ("engine_resources", "engine_resource_count", "engine_resource_set_sha256"),
        ("calibration_artifacts", "calibration_artifact_count", "calibration_artifact_set_sha256"),
    ]:
        entries = manifest[entries_key]
        if not isinstance(entries, list) or not entries or len(entries) != manifest[count_key]:
            raise ValueError(f"invalid {entries_key} count")
        paths = set()
        for entry in entries:
            relative = PurePosixPath(entry["path"])
            if relative.is_absolute() or ".." in relative.parts or "\\" in entry["path"]:
                raise ValueError("resource paths must be relative POSIX paths")
            if entry["path"] in paths or entry["rs_path"] != "data/" + entry["path"]:
                raise ValueError("duplicate resource or inconsistent RS path")
            paths.add(entry["path"])
            for field in ("canonical_sha256", "rs_sha256", "fpw_verified_sha256"):
                if not re.fullmatch(r"[0-9a-f]{64}", entry[field]):
                    raise ValueError(f"invalid {field} for {entry['path']}")
            normalize(b"", entry["normalization"], entry["path"])
        if set_identity(entries) != manifest[identity_key]:
            raise ValueError(f"{entries_key} does not match its recorded set identity")
    return manifest


def check_entries(entries: list[dict], data_dir: Path) -> list[dict]:
    results = []
    for entry in entries:
        row = {"path": entry["path"], "normalization": entry["normalization"]}
        try:
            raw = (data_dir / entry["path"]).read_bytes()
            actual = sha256(raw)
            canonical = sha256(normalize(raw, entry["normalization"], entry["path"]))
            passed = canonical == entry["canonical_sha256"]
            row.update({"passed": passed, "bytes": len(raw), "sha256": actual,
                        "canonical_sha256": canonical})
            if not passed:
                row["status"] = "content_mismatch"
                row["expected_canonical_sha256"] = entry["canonical_sha256"]
            elif actual == entry["rs_sha256"] == entry["fpw_verified_sha256"]:
                row["status"] = "raw_bytes_match_both_repositories"
            elif actual == entry["rs_sha256"]:
                row["status"] = "raw_bytes_match_rs"
            elif actual == entry["fpw_verified_sha256"]:
                row["status"] = "raw_bytes_match_fpw"
            else:
                row["status"] = "text_matches_after_lf_crlf_normalization"
        except FileNotFoundError:
            row.update({"passed": False, "status": "missing"})
        except (OSError, UnicodeError, ValueError) as error:
            row.update({"passed": False, "status": "unverifiable", "reason": str(error)})
        results.append(row)
    return results


def check(manifest: dict, data_dir: Path, include_calibration: bool) -> dict:
    engine = check_entries(manifest["engine_resources"], data_dir)
    calibrations = check_entries(manifest["calibration_artifacts"], data_dir) if include_calibration else []
    results = engine + calibrations
    engine_passed = all(row["passed"] for row in engine)
    calibration_passed = all(row["passed"] for row in calibrations) if include_calibration else None
    return {
        "contract_id": manifest["contract_id"],
        "fpw_reference_commit": manifest["fpw_commit"],
        "scope": "rs_bundled" if include_calibration else "external_shared_engine_directory",
        "data_dir": str(data_dir.resolve()),
        "passed": all(row["passed"] for row in results),
        "engine_resources_checked": len(engine),
        "calibration_artifacts_checked": len(calibrations),
        "engine_resource_set_sha256": manifest["engine_resource_set_sha256"] if engine_passed else None,
        "calibration_artifact_set_sha256": manifest["calibration_artifact_set_sha256"] if calibration_passed else None,
        "missing_or_unverifiable": [row["path"] for row in results if row["status"] in {"missing", "unverifiable"}],
        "unlisted_files_checked": False,
        "excluded": [
            "FPW-owned sRGB.icc and digital LUT payloads",
            "Original full research calibration sources and their derivation metadata",
            "FPW active runtime configuration, application binding and visual acceptance",
        ] + ([] if include_calibration else ["Application-embedded calibrated-output JSON artifacts"]),
        "engine_resources": engine,
        "calibration_artifacts": calibrations,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data-dir", type=Path,
                        help="installed physical resource directory; check the 55 shared resources only")
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT,
                        help="RS checkout to check in default mode")
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--output", type=Path, help="write a detailed JSON result")
    args = parser.parse_args()
    try:
        manifest = load_manifest(args.manifest)
        report = check(manifest, args.data_dir if args.data_dir is not None else args.repo_root / "data",
                       include_calibration=args.data_dir is None)
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f"Invalid resource audit: {error}", file=sys.stderr)
        return 2
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    result = "PASS" if report["passed"] else "FAIL"
    print(f"{result}: {report['engine_resources_checked']} shared engine resources, "
          f"{report['calibration_artifacts_checked']} calibrated-output artifacts ({report['scope']})")
    for row in report["engine_resources"] + report["calibration_artifacts"]:
        if not row["passed"]:
            print(f"  {row['path']}: {row['status']}" + (f" ({row['reason']})" if "reason" in row else ""))
    if report["engine_resource_set_sha256"] is not None:
        print(f"Engine resource identity: {report['engine_resource_set_sha256']}")
    if report["calibration_artifact_set_sha256"] is not None:
        print(f"Calibrated-output resource identity: {report['calibration_artifact_set_sha256']}")
    print("Scope excludes the FPW ICC, digital LUTs, active application binding and visual acceptance.")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
