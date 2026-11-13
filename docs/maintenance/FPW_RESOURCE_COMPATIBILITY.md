# Physical resource compatibility

Resource contract: `fpw-bundled-physical-resources-v1`, audited 2026-11-13.
All 55 shared engine resources match the pinned reference contents: 7 are
byte-identical and 48 differ only by LF/CRLF line endings. Three calibrated-output
JSON artifacts match byte-for-byte. This records repository contents; an installed
host directory needs its own check.

The [per-file record](validation/fpw_resources_20261113.json) preserves exact
hashes and source identities. The
[checker](../../tools/validation/check_fpw_resource_contract.py) runs independently
with Python's standard library.

## Method and scope

The reference is the FPW engine resource snapshot at
`ae3428acd98e39c48784a79dc8f3369ab1236cb1`, compared with RS
`7994b672593c2b8eab697b3d51f22b797dd1cc0d`. For each shared resource, original
bytes or a strict text line-ending reconstruction reproduce both the reference
manifest's SHA256 and its actual Git blob SHA1. The three calibration artifacts
reproduce the reference Git blobs directly.

| Resource group | Files | Byte-identical | Only LF/CRLF differs |
| --- | ---: | ---: | ---: |
| Film and print profiles | 31 | 0 | 31 |
| Filter data and notices | 17 | 1 | 16 |
| Spectral LUTs and Arctic manifest | 7 | 6 | 1 |
| Calibrations and heldout fixture | 3 | 3 | 0 |

Only known UTF-8 `.json`, `.csv`, `.txt` and `.toml` resources may normalize CRLF
to LF. Bare CR is rejected. Every other byte, including a final newline, is
preserved. There is no JSON reserialization, whitespace trimming, numeric
rounding or tolerance. Binary `.npy`/`.lut` resources are compared byte-for-byte.
Existing licenses and data attribution remain part of the resource set.

## Calibration provenance

The preserved `base`/`curves` files have these raw hashes:

| Artifact under `data/negative_scan_ap0_v1/` | SHA256 |
| --- | --- |
| `pro400h.json` | `6728ba04757b352d70756367efd1f2465f2b15ed4d1ba6882a0bef8c589606b3` |
| `xtra.json` | `85db3fc1733979c95a53f622f4ba0cb137fab7eecd2b936529634bad23a69ad8` |
| `heldout.json` | `fd8d58649ce26b31f6c919f6ca4d7d668e2d557aca3ce4e87542a0e2c37b34ce` |

The [calibration provenance](../../data/negative_scan_ap0_v1/PROVENANCE.md)
separately records reported hashes of the original full research sources.
Those originals were unavailable for this audit. Their reported identities are
not hashes of the extracted files above and are not independently verified.
Likewise, a reported upstream import revision is provenance metadata; it does
not by itself establish a verified Git ancestor.

## Check bundled or installed resources

```sh
# Bundled data: 55 engine resources plus 3 calibration artifacts.
python tools/validation/check_fpw_resource_contract.py --output resource-check.json

# An installed engine data directory: 55 shared resources only.
python tools/validation/check_fpw_resource_contract.py \
  --data-dir /path/to/engine_resources \
  --output installed-resource-check.json
```

Missing, unreadable, invalid-text or changed files produce a nonzero exit status.
The checker permits only the line-ending difference described above and never
updates or downloads expected values. It reports its checked scope. Extra
unlisted files, host configuration, display/export ICC profiles and external
digital-look LUTs are outside that scope. A successful check does not certify
host packaging or visual output.

This also catches missing neutral-filter data: the ordinary engine loader can
use an empty database for missing/unreadable input. Qualification should reject
that condition explicitly. A manifest declaration alone cannot prove that the
referenced physical files are present and unchanged.

## Resource identity

Each identity hashes sorted `[path, canonical_sha256]` pairs serialized as compact
ASCII JSON. Canonicalization is limited to the text line-ending rule above.

| Scope | SHA256 identity |
| --- | --- |
| 55 shared engine resources | `f0b0044b1cf96e6945c62556e655079772db11eddd20d50b98eab43d03b22fef` |
| 3 calibration artifacts | `d882ebbea502c51318d9461eae9b93b5b7100cd4cd4f530d2243c1ee36a29478` |

Record engine revision, resource identity, stock-setup version and output route
together. Replacing profiles, LUTs or calibration values requires a new resource
identity, rebuilt calibrated pipelines and fresh numerical acceptance. Reusing
the same directory path does not make an old in-memory pipeline current. See the
[compatibility plan](FPW_PARITY_MIGRATION_PLAN.md).
