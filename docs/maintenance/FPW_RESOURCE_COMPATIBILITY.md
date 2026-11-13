# FPW physical resource compatibility

Audit date: 2026-11-13. Resource contract: `fpw-bundled-physical-resources-v1`.

**All 55 shared engine resource files match the pinned FPW resource contents.**
Seven are byte-identical; 48 differ only by LF versus CRLF line endings. There
are no other text, numeric-token or binary differences in this set. The three
calibrated-output JSON artifacts also match the pinned FPW Git blobs exactly.
This is a repository-content result; the active FPW installation was not read.

The complete per-file hashes and evidence are in
[fpw_resources_20261113.json](validation/fpw_resources_20261113.json).
The check is reproducible without access to FPW using
[check_fpw_resource_contract.py](../../tools/validation/check_fpw_resource_contract.py).

## Sources and method

The comparison fixes FPW at
[`ae3428acd98e39c48784a79dc8f3369ab1236cb1`](https://github.com/ye602a/fursuit-photo-workflow/tree/ae3428acd98e39c48784a79dc8f3369ab1236cb1).
Its [RESOURCE_PIN.json](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/vendor/spektrafilm/data/RESOURCE_PIN.json)
declares 56 SHA256 values. The 55 shared files were compared with RS's existing
`data/profiles`, `data/filters` and `data/luts` contents at
`7994b672593c2b8eab697b3d51f22b797dd1cc0d`. The remaining entry is FPW's
host-owned `sRGB.icc`.

For every shared file, RS bytes, or their strictly LF-to-CRLF text reconstruction,
reproduce **both** the declared FPW SHA256 and the actual Git blob SHA1 returned
for that file at the pinned FPW commit. The comparison therefore checks the
repository objects as well as the manifest declarations. For the three extracted
calibration/fixture JSON files, the actual RS bytes reproduce the pinned FPW
Git blobs directly; their raw SHA256 values are recorded separately.

| Resource group | Files | Byte-identical | Only LF/CRLF differs |
| --- | ---: | ---: | ---: |
| Film and print profiles | 31 | 0 | 31 |
| Filter data and notices | 17 | 1 | 16 |
| Spectral LUTs and Arctic manifest | 7 | 6 | 1 |
| Pro400H/X-Tra extractions and heldout fixture | 3 | 3 | 0 |

FPW's [.gitattributes](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/.gitattributes)
preserves the vendored bytes with `-text`. In this audit the differing FPW text
files contain CRLF and the RS versions contain LF. Equality after removing the
CR in CRLF means exactly the same text and numeric tokens; it does not mean that
the raw SHA256 values are equal. No JSON reserialization, whitespace removal,
CSV parsing, rounding or numeric tolerance is used. The six `.npy`/`.lut` files
are always compared as binary, including the Hanatos spectra used by the current
accepted pipeline. The checker permits only known UTF-8 `.json`, `.csv`, `.txt`
and `.toml` resources to normalize CRLF to LF; it rejects bare CR and preserves
all other bytes, including a final newline.

[FPW_SOURCE.json](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/vendor/spektrafilm/FPW_SOURCE.json)
states that the vendor source was archived from
`turbasvin/spektrafilm-rs@27ca80e3edab27f99abcc8c2860f5cc3617775f3`, with listed
FPW changes. That is a pinned provenance statement. This audit verifies the FPW
tree and does not infer the current RS fork's commit ancestry from the statement.
Existing profile attribution, CC BY-SA notices, spectral-data notices and source
licenses remain part of the resources; none were rewritten for this comparison.

## Calibration hashes have different scopes

The pinned [negative_scan.rs](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src/negative_scan.rs)
maps `fujifilm_pro_400h` to `look_data/pro400h.json` and `fujifilm_xtra_400` to
`look_data/xtra.json`. Those mappings and the associated bundled profile files
are verifiable. The RS files under `data/negative_scan_ap0_v1` contain the same
extracted `base`/`curves` as the pinned FPW blobs.

| RS artifact | Verified raw SHA256 |
| --- | --- |
| `negative_scan_ap0_v1/pro400h.json` | `6728ba04757b352d70756367efd1f2465f2b15ed4d1ba6882a0bef8c589606b3` |
| `negative_scan_ap0_v1/xtra.json` | `85db3fc1733979c95a53f622f4ba0cb137fab7eecd2b936529634bad23a69ad8` |
| `negative_scan_ap0_v1/heldout.json` | `fd8d58649ce26b31f6c919f6ca4d7d668e2d557aca3ce4e87542a0e2c37b34ce` |

The [FPW provenance note](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src/look_data/PROVENANCE.md)
reports different hashes for the **original full research calibration sources**:

- Pro400H: `7b071cf217f305099908266e6285c7dcb3856f42f657fa99b3acac913ca0457f`.
- X-Tra: `7a8dfd7d8dd24739cea8bbd02794f785a0cb2632082a38e102af6dce3aed15a1`.

These are neither profile JSON hashes nor hashes of the extracted `base`/`curves`
files above. FPW says the full original metadata/results remain in the research
worktree and refers to `docs/fuji_select/INTEGRATION_REFERENCES.json`. Those
original files were unavailable here, so their hashes and the full derivation
cannot be independently reverified. The machine-readable record labels them as
provenance-reported, unverified full-source references. It does not relabel them
as verified source-profile hashes.

## Installed-resource gap and the narrow checker

[Initialize.ps1](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/tools/native/Initialize.ps1)
points a new installation at `native/vendor/spektrafilm/data` and combines the
engine identifier with a canonical hash of `RESOURCE_PIN.json`. At the pinned
FPW revision, [Config::load in images.rs](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src/images.rs)
checks that declared manifest identity against `engine_contract`; it does not
iterate over the physical files and rehash their payloads. Its resource-load
test deliberately succeeds with the manifest present and the physical resources
absent. A valid declaration alone therefore cannot establish that an installed
directory still contains the accepted bytes.

The active `runtime-config.json` and installed directory were not available in
this workspace. The read-only checker can close the shared-content part of that
gap on the actual FPW host:

```bash
# RS repository: 55 shared engine files plus 3 calibrated-output artifacts.
python tools/validation/check_fpw_resource_contract.py --output resource-check.json

# Actual engine_resources directory from the FPW runtime configuration:
# 55 shared files only; embedded calibration artifacts are outside this mode.
python tools/validation/check_fpw_resource_contract.py \
  --data-dir /path/to/actual/engine_resources \
  --output installed-resource-check.json
```

The script uses Python's standard library. It reports every missing,
unreadable/invalid-text or changed resource and returns a nonzero status. It
accepts a Windows text checkout when only CRLF differs. Both `.npy` and `.lut`
newline mutation tests fail as required. It does not change, generate, download
or silently accept new resource values. The JSON result identifies the checked
scope; extra unlisted files, the active application's configuration binding and
its visual output are not checked.

This check also makes missing neutral-filter data explicit: the existing RS
`NeutralFilters::load` treats missing or unreadable JSON as an empty database.
The resource checker rejects that condition before an FPW qualification run;
ordinary standalone engine defaults are unchanged.

The `sRGB.icc` entry remains FPW-owned. Its pin is
`f869f25625302eef46cd74fe56ec11adbfa7cf20db6f70b8efdddcbeec4323a0`; the pinned
tree lists blob `f0e82d06cb32ba25e7b82ca93d5c2d64b645734f` (588 bytes).
Its payload was not fetched/rehashed in this audit and was not copied into RS.
FPW's `images.rs` uses it for JPEG ICC embedding. Both checker modes exclude this
host output resource, so their success does not certify FPW export packaging.

The six digital-look LUT hashes are declared in the pinned
[look-data catalog](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src/look_data/catalog.json),
but their payloads are external, as documented by
[FUJI_INTEGRATION.md](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/FUJI_INTEGRATION.md).
The audit records those references as unverified and outside the shared physical
engine resource set. It adds no digital LUT or wide-gamut implementation.

## Minimum identity for independent RS updates

The record gives the shared resource set a content identity separate from the
calibrated-output artifacts. Both use sorted `[path, canonical_sha256]` pairs
serialized as compact ASCII JSON, then SHA256; canonicalization is limited to
the explicit text line-ending rule above.

| Scope | Content identity |
| --- | --- |
| 55 shared engine files | `f0b0044b1cf96e6945c62556e655079772db11eddd20d50b98eab43d03b22fef` |
| 3 calibrated-output artifacts | `d882ebbea502c51318d9461eae9b93b5b7100cd4cd4f530d2243c1ee36a29478` |

For an FPW acceptance record, pin the RS source revision, these resource-set
identities, the selected stock-setup version, and the existing output-route
version together. Record the actual installed-directory checker result alongside
the pixel/interaction results. A filesystem path alone is not a resource
version. Replacing resources requires rebuilding the calibrated pipeline;
keeping the same path does not make an old in-memory pipeline current.

The historical FPW contract is
`27ca80e3edab27f99abcc8c2860f5cc3617775f3:29e4e607bc26a83ad9ba95e76d116d844c647d2b0a9d85138a75c794d5ec6e3a`.
It hashes the original declared mapping, including FPW's ICC and original raw
text hashes. It must not be repurposed to claim a different RS source revision
or a newly packaged resource set. The new audit identities are comparison
records; this change does not rewrite any saved FPW recipe or runtime contract.

Preserve the current resources and versioned look contracts during the dependency
handoff. Any deliberate profile/LUT/calibration change needs an explicit new
resource identity and a fresh FPW comparison. The frozen six-photo outputs,
device-specific execution, cancellation/recovery and real editing interactions
remain separate gates in [FPW_PARITY_MIGRATION_PLAN.md](FPW_PARITY_MIGRATION_PLAN.md).
