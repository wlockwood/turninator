# Task 8b and 9b inline integration checkpoint

Root implemented this continuation without additional agents, at the user's request. This is author self-review; no independent review is claimed.

The production host adapter strictly binds the compiled package contract to all ten owned runtime hashes and the actual SDK, compiler, OpenMP, OIDN and two loaded driver files. Dynamic observations and paths are excluded from immutable identity. The application owns a !Send runtime cache on its export actor, retaining a scene-free idle SDK context across independent job contexts. No Rust TLS destructor owns SDK state. Backend discovery remains lazy, unavailable packages/devices fail without substitution, and common geometry/material/export code handles every layout.

The deterministic positive constructor seed derives from SHA256 of the complete immutable pixel recipe. Native diagnostics record its effective value and minimum sample counts. SDK session startup/edit, render wait, denoising and readback/color observations are now nested first-frame/later-frame report timings; startup includes compilation and is not labeled isolated GPU time. Existing Filament metadata without those fields remains valid. No native pixel policy changed in this host continuation.

Explicit approximate support remains for transparent resin RGBA and denoised fine-layer animation. Both preflight assessment and actual export reports expose actionable guidance, including four-view resin alpha even when the request's raw alpha flag is false. Selecting a material never switches engines.

The export actor acknowledges context_released only after SDK teardown outside the queue lock. The interactive Filament viewport waits for that acknowledgement before resuming. The slow-destructor regression first returned Ok(true) incorrectly, then passed with Ok(false) during teardown and true afterward, while UI queue calls stayed responsive.

Observed failures retained: receipt/seed RED tests; actual TLS smoke rendered but hung on thread exit; initial Windows verbatim DLL-path attempts failed with os126. Explicit lexical ownership and short/unambiguous canonical-to-DOS loader spelling fix those cases. Ambiguous/long/UNC paths retain verbatim behavior; ownership/hash checks still use canonical paths. No system libraries or process-wide PATH were modified.

Verification from artifacts/root-integration and artifacts/luxcore-host:

- Receipt/seed/support-policy unit checks4 plus seed1 pass; discovery1 pass.
- handoff-green: embedded_loader14, lifecycle3, UI9, project migration4, requirements2 pass.
- smoke-v4: actual cancellation/distinct-token reuse1upload/identity and clean process exit PASS15.57s.
- surface-v1: frozen512 analytical mask IoU1/count102400, 14poses A-B-A within2codes/oneupload, positive seed/minimum64,4096 CPU RGBA67108864 bytes, empty stage, posed mapped layer shader, cancellation/drain/reuse, quality mismatch rejection, independent contexts/recreate. Common transparent still/opaque preview/sheet/paired section/GIF/build-up MP4 pass, retained reruns byte-identical. Total127.95s. Functional timings include other CPU work and are not controlled benchmark results.
- owner-v1: twenty actual independent Rust backend contexts share one SDK owner, then paired-section camera/build timeline MP4. Dropping the last backend/cache unloads bridge/SDK/NVRTC modules; clean exit121.42s. Native8a separately proves GPU0 after every context destruction and bounded handles/private memory.
- ui-v1: actual painted normal/narrow/resin-alpha/sheet/LuxCore-to-Filament handoff/Filament export resume6cases pass56.56s. Resin approximation warning visible; canonical camera and selected renderer retained. Owned Job Object total_processes1/no children/exit0/active0, tools deliberately absent. Root viewed resin and handoff screenshots.
- Fresh configured full Rust suite:349 passed/59 ignored/53 suites, exit0 (test-luxcore-integration.log). Actual GPU tests above were executed explicitly, not counted as default-suite passes.
- Strict Clippy passed before final observational timing addition; final whole-branch checks remain required.

Final five-engine matched benchmark, extracted full/core packages, actual missing-runtime/device desktop cases and print-geometry matrix remain Task10 qualification gates. This checkpoint does not claim complete Task10 or support on untested hardware.
