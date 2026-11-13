"""Materialize a build from authorized, pinned local checkouts; no network/auth."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil


def digest(data):
    return hashlib.sha256(data).hexdigest()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--fpw-vendor", type=Path, required=True,
                        help="ae3428a checkout's native/vendor/spektrafilm directory")
    parser.add_argument("--rs-root", type=Path, required=True,
                        help="candidate c0308a checkout")
    parser.add_argument("--work-dir", type=Path, required=True)
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    work = args.work_dir.resolve()
    work.mkdir(parents=True, exist_ok=True)
    if (work / "baseline").exists():
        raise SystemExit("Choose a fresh work directory; existing baseline is not overwritten")
    manifest = json.loads((here / "source-blobs.json").read_text())
    for row in manifest["files"]:
        data = (args.fpw_vendor / row["path"]).read_bytes()
        blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        assert blob == row["git_blob"] and digest(data) == row["sha256"], row["path"]
        target = work / "baseline" / row["path"]
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    # The only reference workspace edit: omit members that are not compiled.
    cargo = work / "baseline/Cargo.toml"
    text = cargo.read_text().replace('    "crates/spektrafilm-cli",\n', "")
    text = text.replace('    "crates/spektrafilm-gui",\n', "")
    cargo.write_text(text)
    candidate_manifest = json.loads((here / "candidate-sources.json").read_text())
    for row in candidate_manifest["files"]:
        assert digest((args.rs_root / row["path"]).read_bytes()) == row["sha256"], row["path"]
    for row in json.loads((here / "resources-55.json").read_text()):
        data = (args.rs_root / "data" / row["path"]).read_bytes()
        assert digest(data) == row["rs_sha256"], row["path"]
        target = work / "data" / row["path"]
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    for name, engine in [("fpw", work / "baseline"), ("rs", args.rs_root.resolve())]:
        probe = work / ("probe-" + name)
        (probe / "src").mkdir(parents=True)
        shutil.copyfile(here / "probes" / (name + ".rs"), probe / "src/main.rs")
        manifest = (here / "probes" / (name + ".Cargo.toml")).read_text()
        manifest = manifest.replace("${ENGINE_ROOT}", engine.as_posix())
        (probe / "Cargo.toml").write_text(manifest)
        shutil.copyfile(here / "locks" / (name + ".Cargo.lock"), probe / "Cargo.lock")
    print(json.dumps({"work_dir": str(work), "reference_blobs": len(json.loads((here / "source-blobs.json").read_text())["files"]),
                      "resources": 55, "numeric_source_changes": 0}))
