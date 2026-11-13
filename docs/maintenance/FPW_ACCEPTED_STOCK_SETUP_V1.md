# FPW accepted stock setup v1

Date: 2026-11-13. Contract identifier: `fpw-accepted-stock-v1`.

FPW is the primary product driving this compatibility contract. RS owns the
film-engine constants so an FPW dependency update can consume one maintained
implementation. The public engine can also be used independently. No generic
preset framework or replacement visual style is introduced.

## Exact source and scope

The stock constants and their ordering come from
[`Engine::configure_pipeline` in FPW](https://github.com/ye602a/fursuit-photo-workflow/blob/ae3428acd98e39c48784a79dc8f3369ab1236cb1/native/crates/fpw-runtime/src/film.rs),
revision `ae3428acd98e39c48784a79dc8f3369ab1236cb1`, Git blob
`ba88953cafe93cbc1eedf0c2582ec19a34ba57f3`. This extraction retains that source
attribution under the RS repository's GPL-3.0 license. Existing film-profile,
spectral-data and upstream implementation notices remain unchanged. No FPW UI,
catalog, application worker, recipe object or private photographic asset is copied.

`AcceptedStockSetup::FpwV1(StockRoute::{Paper, Slide, NegativeScan})` explicitly
selects these existing stock defaults. `Pipeline::new_with_spectral_and_stock_setup`
constructs a calibrated pipeline with the selection, and `accepted_stock_setup()`
reports it. Existing `Pipeline::new`, `new_with_spectral`, and
`RuntimeParams::default()` do not select this contract or change behavior.

The selection covers **stock setup only**. The host still supplies its accepted
exposure, grain controls, source sampling and output-route parameters. In
particular, selecting `Slide` or `NegativeScan` here does not set `io.scan_film`,
color spaces, transfer functions, scanner inversion, or a complete FPW recipe.
The existing separate negative-scan transport is not a new wide-gamut migration.

## Preserved branches

Before the ordinary spectral constructor runs:

| Profile metadata | Accepted assignment |
| --- | --- |
| `use == "cine"` | `halation_first_sigma_um = [50, 50, 50]` |
| Other `use` values | `halation_first_sigma_um = [65, 65, 65]` |
| `antihalation == "strong"` | `halation_strength = [0.015, 0.005, 0]` |
| `antihalation == "weak"` | `halation_strength = [0.08, 0.02, 0]` |
| `antihalation == "no"` | `halation_strength = [0.30, 0.10, 0.015]` |
| Unknown antihalation value | Keep the caller's strength values |

The original route check is preserved: a positive profile is accepted only for
the `Slide` route, and `Slide` requires a positive profile. For slide setup,
the profile's canonical `info.stock` must be `fujifilm_provia_100f` (scale `1.3`)
or `fujifilm_velvia_100` (scale `0.9`). Other positive stocks, including
Ektachrome, Kodachrome and the current Tri-X positive profile, are rejected by
this **versioned compatibility contract**, not removed from the ordinary engine.
The bundled profiles' canonical IDs match the resource names used by the source
adapter; unverified stock aliases are not added to the accepted set.

After the spectral constructor has completed, the accepted slide scale `s` sets:

- Same-layer DIR RGB: `[0.12*s, 0.08*s, 0.06*s]`.
- Red-to-green/blue: `[0.12*s, 0.06*s]`.
- Green-to-red/blue: `[0.08*s, 0.06*s]`.
- Blue-to-red/green: `[0.06*s, 0.06*s]`.

The DIR enable flag, amount and other recipe controls are preserved. Assignments
use the original f64 operation order and write fixed values, so repeated runtime
updates do not compound the scale.

## Initialization and caching

The required order is: validate/resolve stock → apply metadata halation defaults
→ ordinary `new_with_spectral` calibration → apply accepted slide DIR values.
The ordinary constructor's existing B&W channel-0 normalization runs between the
two steps. A calibrated pipeline retains the resolved version/stock selection.
`with_params` reapplies halation before its ordinary film-specific normalization
and slide DIR after it, preserving both slide values and B&W broadcasting.

This does not expand `with_params` into a recalibration API. Its existing rule
still applies: changes that affect spectral calibration require a new pipeline.
For runtime-only edits, start from the calibrated `pipeline.params.clone()` so
resolved neutral filters and other calibration inputs are retained.
Changing profile or accepted setup also requires a new pipeline. A host retaining
calibrated pipelines includes the setup version and route in that pipeline's key.

The GPU density key already covers the prepared halation and DIR parameters.
The accepted values therefore participate in RS's existing exact stage-dependency
comparison without a duplicate FPW invalidation table. Merely changing a version
label while producing identical prepared physics does not require a different
physical negative; the constructor/profile lifecycle remains the host boundary.

## Verification boundaries

`stock_setup` unit tests cover every metadata branch, both accepted slide stocks,
unsupported/missing stocks, route mismatches, idempotence and all bundled filming
profiles. Integration tests compare the opt-in constructor to independently
transcribed statements from the pinned FPW adapter, including full resolved
parameters, Hanatos LUT bits, enlarger illumination, preflash and print exposure
calibration. They also verify unchanged ordinary constructors and runtime updates
for Provia, Velvia and B&W Double-X.

These are implementation and calibration parity checks. The frozen six-photo
FPW comparison, host recipe integration, RTX/AMD execution and real editing
interaction gates remain in [FPW_PARITY_MIGRATION_PLAN.md](FPW_PARITY_MIGRATION_PLAN.md).
FPW itself is unchanged during this extraction. Any later visual retuning must
receive a new contract version and its own acceptance evidence.
