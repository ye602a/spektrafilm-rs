# Versioned negative-scan calibration

`pro400h.json` and `xtra.json` contain frozen base/curve arrays derived from
Andrea Volpato's spektrafilm profiles. `heldout.json` contains sparse reference
values evaluated with the original Python inversion/display functions; it is
an independent validation fixture, not a replacement calibration.

The three files were copied byte-for-byte from the accepted FPW reference at
commit `ae3428acd98e39c48784a79dc8f3369ab1236cb1` on 2026-11-13. FPW is a
photography application that is not yet public. Its application sources and
research assets are not required to use these bundled files.

| File | Original Git blob | SHA-256 |
| --- | --- | --- |
| `pro400h.json` | `66f97e447fffa0fd340856af33305b919d4c0e90` | `6728ba04757b352d70756367efd1f2465f2b15ed4d1ba6882a0bef8c589606b3` |
| `xtra.json` | `31b5b29139b890f6a7c1b6f29d13bbca24f634e8` | `85db3fc1733979c95a53f622f4ba0cb137fab7eecd2b936529634bad23a69ad8` |
| `heldout.json` | `9de31bf081a2415e29c30956af1d62d989eced1c` | `fd8d58649ce26b31f6c919f6ca4d7d668e2d557aca3ce4e87542a0e2c37b34ce` |

The original provenance records these SHA-256 identities for the **full research
calibration sources**:

- Pro400H: `7b071cf217f305099908266e6285c7dcb3856f42f657fa99b3acac913ca0457f`
- X-Tra: `7a8dfd7d8dd24739cea8bbd02794f785a0cb2632082a38e102af6dce3aed15a1`

Those full sources are not bundled or independently reverified here. Their
identities differ from the extracted JSON hashes above; they must not be used
as interchangeable checksums.

The persisted `negative_scan_ap0_v1` identity binds this calibration and the
existing linear-AP0 transport/inversion contract. Table or algorithm changes
require a new route version. No fitting, node resampling, or regenerated
heldout data was introduced by the extraction or documentation cleanup.

Profile-derived data retains Andrea Volpato's CC BY-SA 4.0 attribution and
share-alike terms. See the full
[upstream data license](../../SPEKTRAFILM_DATA_LICENSE.txt) and
[calibrated output contract](../../docs/maintenance/CALIBRATED_OUTPUT_V1.md).
`python scripts/verify_calibrated_output_sources.py` verifies these files and
the frozen shader/reference sources offline.
