# Timeline project persistence and settle UI review

## Tests

`tests/project_timeline.rs` exercises real project save/load, without a renderer or GPU:

- A two-key spiral with smooth easing, camera pan/zoom, turntable and build-up survives a project round trip. Hand-derived midpoint yaw 540 degrees, pitch 30 degrees, target [5,10,15], distance 200 mm, turntable 180 degrees and build fraction 0.5 verify playback semantics. Saved section position and presentation rotation also survive.
- Version-1 projects omitting either timeline or the entire output-options object remain loadable with the previous Turntable behavior.
- Both loading and saving reject eleven invalid timeline cases: invalid duration, empty/excess keys, duplicate/nonzero-first/out-of-range times, mixed projections, invalid camera zoom, fractional spiral turns and a discontinuous loop endpoint.
- Invalid saves preserve the bytes of an existing valid project. NaN and both infinities in a timeline camera are rejected before JSON can turn them into an unreloadable value.

The first attempts were blocked by the root's intermediate UI compilation state, not a demonstrated behavioral RED. Once integrated, an initial parallel test failed because its temporary directory disappeared. A diagnostic rerun passed; timestamp collision was a plausible fixture weakness, so a process-local atomic suffix now makes all concurrent fixture directories distinct. These tests are regression coverage added alongside the root implementation; no production code was changed by this subtask.

`cargo test --offline --test project_timeline`: 5 passed. `cargo clippy --offline --test project_timeline -- -D warnings`: passed. `cargo test --offline --all-targets`: 81 passed, 15 explicitly ignored runtime/GPU/proof tests, no failures.

## Settle UI review

The main geometry contract is integrated correctly: the importer builds and retains an `Arc<PickingMesh>` off the UI thread, including whole-layer face visibility metadata; the click handler passes the native response rectangle, active camera and viewport settings into `pick_at`; `settle_hit` uses the original build normal, updates only presentation rotation, resets turntable spin, fits the displayed bounds and invalidates the preview. The native viewport is always visible in its own column, so encoded-output visibility does not prevent picking.

Two small remaining issues were sent to the parent for correction (this review does not edit app code):

1. Entering or cancelling face selection resets `timeline_angle` without invalidating the preview. With a paused nonzero timeline angle and an up-to-date rendered preview, native geometry changes while that old rendered preview remains marked current. Call `changed()` when the angle actually changes.
2. The face-click handler does not guard `!importing`. Debounced settings changes and opening a project can mark importing before `import()` clears face mode. A click in that interval can settle the previously displayed mesh. Guard the handler or clear selection immediately when those import intents start.

No new GPU work was performed in this subtask. Previous explicit GPU picking/settling tests remain the lower-level evidence; desktop click-mode and project restoration validation belong to the parent's live smoke.

## Follow-up: sampled stage and final integration

The parent fixed both settle issues above. Inspection confirms that the shared `settle_pointer` rejects importing state and selection-mode angle resets invalidate the preview. The parent's `artifacts/ui-timeline-smoke/smoke/result.json` records all Boolean checks true, including actual native picking/settling, timeline project restoration, decoded four-frame timeline output, sampled native pose and sampled preview request matching; error is null and preview/current revisions are both 13.

Project coverage now has eight tests. Added cases preserve 0, 17 and 50000 visible layers and signed/fractional sampled angles, with or without a timeline. Older version-1 files missing the new fields default to no fixed stage and zero spin. Loading rejects excessive/negative/fractional layer counts and invalid sampled angles; saving rejects excessive layer counts, out-of-range angles and NaN/infinities while preserving a prior valid file.

Final app review found these additional issues and sent them to the parent (app/project remain parent-owned):

- Build-height controls and standalone build playback need revision invalidation now that preview requests include fixed build stages; otherwise native changes leave the rendered stage marked current.
- A G-code timeline's deposited build-up flag must not survive as an active but disabled option on a plain STL. Sampling an empty layer schedule can hide the entire native model, and export attempts unsupported deposited build-up.
- Pending restored stage must be scoped to its project import, cleared on failure/supersession, and standalone build playback stopped on project restore. Otherwise another later model can consume the old stage or an immediately running playback can overwrite the restored one.
- Explicit sidebar camera actions (Fit, canonical views and Orthographic) should pause timeline playback just as native camera dragging does, so the next playback tick does not overwrite the user's camera action.

`cargo test --offline --test project_timeline`: eight passed after the extension. These are regression additions against the parent's existing implementation, not a new production RED/GREEN claim.

Follow-up verification: targeted strict Clippy passed. The subsequent full-suite run stopped at the concurrently added `session::snapshot_bytes_count_toward_quota_before_any_snapshot_write` regression: it expected a budget error but received `Blender executable does not exist`. All preceding suites, including all eight project timeline tests, passed. This was reported to the parent as an in-progress quota-test failure; this review does not claim that later full run was green.

Inspection of the parent's first import-reset fix also identified a regression to avoid: resetting the authored timeline before an import succeeds loses it when the new model fails to load and the previous model remains displayed. Reset model-specific animation state on accepted new-source imports, or restore it on failure. New-source intent (including the Recent menu while another import is pending) should be explicit rather than inferred from `restore_camera`, which also serves project/reimport framing.
