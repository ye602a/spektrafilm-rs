# Compatibility and host adoption

Maintain one film implementation in RS, with independently buildable library,
CLI and GUI targets. Hosts adopt an exact accepted revision and matching
resources so ongoing engine development does not silently change saved results.

## Versioned rendering

Preserve the current linear-sRGB input convention and existing output routes.
Changes to calibrated tables, accepted defaults, operation order or working
space require a new rendering-contract identity and replay of saved settings.
Wider-gamut processing remains separate research.

The [host GPU contract](HOST_GPU_INTEGRATION_CONTRACT.md) covers device injection,
source-anchored grain, precise IIR, bounded caches and final output. Stock setup
and calibrated output are explicit selections. `gpu_multilayer` remains `false`
by default for standalone compatibility; a host adopting an existing multilayer
route must explicitly enable it.

## Adoption checklist

1. Bind the source revision, verified resource identity, stock-setup version and
   output route together. Use the public APIs for film processing and cache
   dependencies; retain application state and scheduling in the host.
2. Check the actual installed resource directory with the
   [resource checker](FPW_RESOURCE_COMPATIBILITY.md). A matching manifest name
   or directory path alone does not establish unchanged resource contents.
3. Compare accepted images on the same device and driver, with identical input
   pixels, exposure, profiles, physical effects, source transform and output
   sampling. Check saved-settings replay, preview/export agreement and region
   output as well as isolated operators.
4. Exercise real shared-device surface changes, overlapping requests, stale-frame
   rejection, submitted cancellation and device recovery. Headless software
   Vulkan tests do not establish window-system or dedicated-hardware behavior.
5. Record acceptance evidence and retain a rollback revision before replacing
   an existing host dependency. Remove migrated algorithm copies after adoption.

### Reduced CPU negative output

The historical reduced CPU fallback can resize transmission before nonlinear
negative inversion and apply finish after reduction. The calibrated GPU v1
route inverts, quantizes and applies finish before crop/reduction. These orders
are not equivalent. Keep the saved route's behavior explicit during adoption;
see [CALIBRATED_OUTPUT_V1.md](CALIBRATED_OUTPUT_V1.md). The full-size CPU inversion
APIs do not implement the old reduced fallback implicitly.

## Qualification and performance

[VALIDATION.md](VALIDATION.md) records completed standalone checks. The precise
Vulkan IIR suite has 1,215 cases and requires
`abs(GPU - reference) <= 4*f32::EPSILON*max(1,abs(reference))`.
Run it on each target device/driver that needs qualification; software Vulkan
success does not certify every GPU backend.

Current allocation reuse retains one pair of work planes, while density reuse
retains one immutable snapshot. Auxiliary blur/LUT/parameter buffers and staging
are not a general resource pool. Measure real workloads, including exact-key
construction, GPU copies, cold/warm state and active memory, before claiming a
performance gain. Cache residency counters do not represent total VRAM usage.
