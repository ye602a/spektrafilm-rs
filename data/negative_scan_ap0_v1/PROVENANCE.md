# Frozen integration data, 2026-10-23

Source: FPW-FUJI-SELECT-01 docs/fuji_select/INTEGRATION_REFERENCES.json.
All five code references and two calibration source hashes verified before integration.
`catalog.json` contains identities and source restrictions only; no colour LUT samples.
`pro400h.json` and `xtra.json` retain the original base/curves arrays without fitting or resampling, derived from the pinned spektrafilm profiles. Preserve upstream Andrea Volpato CC BY-SA 4.0 profile attribution; see native/vendor/spektrafilm data licenses. The original metadata/results remain in the research worktree.
`heldout.json` is a sparse extraction from the frozen heldout transmission scan, evaluated with the original Python invert_linear/positive_display functions. It is an independent parity fixture, not a replacement calibration.
Negative v1 requires the pinned engine contract, original DIR active, fixed spectral settings and no diffusion. Exposure and physical texture controls remain separate. Scan glare off, linear AP0 transport, output gamut off, no transport sharpen; inversion/TOE/shoulder/gamut happen once in Rust.
No FujiSim module, build-time external path injection, Lab DI or green protection is included. The later digital GPU integration adds only a borrowed device/queue accessor to the vendored backend; physical kernels and profile data remain unchanged.

Persisted route `negative_scan_ap0_v1`, together with film profile and engine identity, binds this fixed mapping: Pro400H calibration source SHA256 7b071cf217f305099908266e6285c7dcb3856f42f657fa99b3acac913ca0457f; X-Tra 7a8dfd7d8dd24739cea8bbd02794f785a0cb2632082a38e102af6dce3aed15a1. Any algorithm/table change requires a new route version; never silently replace v1. Slide-specific setup is separately persisted as slide_v1.

Digital GPU colour code derives from the frozen FPW research kernel, with the unaccepted grain branch removed. Actual LUT files remain external; see native/FUJI_INTEGRATION.md for personal verification terms and distribution restrictions.


## Spektrafilm-rs extraction, 2026-11-13

The three files in this directory are copied byte-for-byte from
[FPW commit ae3428acd98e39c48784a79dc8f3369ab1236cb1](https://github.com/ye602a/fursuit-photo-workflow/tree/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src/look_data).
The original provenance above is retained. References above to catalog, digital
LUTs and FPW research worktrees describe their source context; those assets are
not part of this extraction.

| Local file | Original Git blob | Local SHA-256 |
| --- | --- | --- |
| `pro400h.json` | `66f97e447fffa0fd340856af33305b919d4c0e90` | `6728ba04757b352d70756367efd1f2465f2b15ed4d1ba6882a0bef8c589606b3` |
| `xtra.json` | `31b5b29139b890f6a7c1b6f29d13bbca24f634e8` | `85db3fc1733979c95a53f622f4ba0cb137fab7eecd2b936529634bad23a69ad8` |
| `heldout.json` | `9de31bf081a2415e29c30956af1d62d989eced1c` | `fd8d58649ce26b31f6c919f6ca4d7d668e2d557aca3ce4e87542a0e2c37b34ce` |

The calibration-source hashes in the original provenance identify the full
research source files. The hashes in this table identify the frozen extracted
JSON files used by the engine; they intentionally refer to different artifacts.

Profile-derived data retains Andrea Volpato's CC BY-SA 4.0 attribution and
share-alike terms. The full upstream data license is preserved at
[`SPEKTRAFILM_DATA_LICENSE.txt`](../../SPEKTRAFILM_DATA_LICENSE.txt).
No fitting, node resampling, regenerated heldout data or calibration changes
were made during this migration. Keep these exact bytes under the v1 identity.

Implementation and independent reference-test provenance are documented in
[`CALIBRATED_OUTPUT_V1.md`](../../docs/maintenance/CALIBRATED_OUTPUT_V1.md).
`python scripts/verify_calibrated_output_sources.py` checks these hashes and the
corresponding frozen shader/reference source files offline.

