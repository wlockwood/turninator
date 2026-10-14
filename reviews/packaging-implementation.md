# Portable packaging and dependency notice evidence

Owned implementation: `tools/package.ps1`, `tools/generate-notices.py`, `tools/license-supplements.json`, `THIRD-PARTY-NOTICES.md`, and `tests/test_packaging.py`. Root retains source/UI/README ownership and commits. No application license was added or changed.

The portable Windows x64 package contains only the selected profile's `turninator.exe`, executable-relative `renderer/worker.py` and `renderer/scene.py`, a portable README, source README, runtime support document, third-party notices, and a version manifest. External Blender/FFmpeg/ffprobe executables remain separate installations. The README describes Tools / storage configuration, persistent data, explicit alternate `--data-dir`, and the tested runtime document.

`tools/package.ps1` defaults to an offline locked release build with the native Windows MSVC toolchain, then creates a unique folder and ZIP under `artifacts/releases`. It never copies a target directory tree. A single safe package name is required; existing destinations and reparse-point output ancestors are rejected. Temporary metadata is removed with a literal, prevalidated workspace path. No recursive deletion is used. A `-SkipBuild` package explicitly records that the packaging command did not build its executable, so its source hashes are not asserted to establish that prebuilt binary's provenance.

The manifest hashes each payload file, the selected source/scripts/Cargo lockfile, compiler identity, resolved dependency versions, and external executable identities available on the packaging machine. External tools are marked `bundled:false`. Package metadata excludes local Cargo-cache paths. Manifest/ZIP hashes are not self-referential.

Notices use the resolved dependency closure from `cargo metadata --offline --locked --filter-platform x86_64-pc-windows-msvc`: 176 exact third-party package versions, conservatively including build dependencies. Cached source license/notice files and bundled OFL/UFL/Hack/emoji font texts are preserved. Several published crate archives omit their own license files. Supplements for sixteen packages (nine distinct upstream files) were retrieved from exact upstream commits recorded in each archive's `.cargo_vcs_info.json`; the checked-in supplement stores URL, original attribution/text and SHA-256. Generation performs no network requests and fails on missing text or mismatched supplemental hashes. The regenerated notices do not choose a license for Turninator or the user's models.

TDD and checks:

- Initial packaging test run failed because `generate-notices.py` was absent. Implemented dependency closure, notice/font extraction, missing-license failure, supplement hash verification, and inventory hashing; the first four tests passed.
- Changed the fixture to `LICENSE.MIT`, matching wgpu's actual cached filenames: observed the test fail because the initial suffix filter omitted it; corrected license discovery and observed green.
- Added traversal test before `package.ps1`: observed failure; implemented the safe output path check and observed green.
- `python -m unittest discover -s tests -p "test_*.py" -v`: **36/36 passed**.
- `git diff --check` for the owned packaging paths passed.
- `powershell -NoProfile -File tools/package.ps1 -SkipBuild -Profile debug -PackageName packaging-validation-20261014`: produced an actual diagnostic package and **16,111,270-byte ZIP**. All seven payload file hashes and exact ZIP file membership verified; the eighth file is the manifest. Only `turninator.exe` is an executable. Repeating the command rejected the existing destination before copying/building. This diagnostic package is explicitly marked prebuilt and is not the final release artifact.

Release build and relocated fresh-settings GUI/renderer smoke remain to be recorded after root finishes the UI integration changes and releases the GPU slot.

## First optimized package: diagnostic, superseded before final release

The optimized locked release build completed in 49.24 seconds. Package `artifacts/releases/turninator-0.1.0-windows-x64-20261014-154251-233ca9e0` was built with Blender 4.5.14 recorded in the manifest. Its ZIP SHA-256 is `74434326e63aeb757fbb242cfe8dde451aef47bed22389ea26e9a2ce670e0192`.

The packaged executable ran from a separate working directory containing spaces, using a fresh data directory with only external tool paths configured. The actual GUI smoke produced `artifacts/packaged-smoke-20261014-154251/smoke/result.json`: all five checks true (stale preview rejection, latest import selection, cancellation, failed-reimport settings preservation, project round-trip), `error:null`, matching scene/preview revision 6, and 107,970 imported triangles. Native and rendered UI screenshots were saved; visual inspection confirmed the framed model, rendered preview and .30/.25 mm settings without an error banner.

An extra wrapper assertion requiring a CIM process command-line observation failed because `Get-CimInstance` is denied in this sandbox. This did not fail the application smoke; its five explicit checks all succeeded. The next wrapper records that observer limitation separately. Executable-relative resource presence and file hashes were verified, and the application selects those resources before its development fallback.

Root's independent review identified additional UI state fixes after this build began. This package is therefore diagnostic and superseded; a fresh incremental package and smoke run are required after those fixes.

## Final package and relocated GUI verification

After root's second source-stability signal, `tools/package.ps1` rebuilt the optimized application in **7.68 seconds** and created:

- Folder: `artifacts/releases/turninator-0.1.0-windows-x64-FINAL-20261014-155450`.
- ZIP: `artifacts/releases/turninator-0.1.0-windows-x64-FINAL-20261014-155450.zip`, **6,599,160 bytes**; SHA-256 `35f99b3b76cb9204609c0d25ec96c9ce2e7c8dd7b9b4fbc0f6aad15d09cbdb7e`.
- Executable: **16,154,112 bytes**; SHA-256 `e92f46ca76d3e48659b7775c74282e9b84ae6bc0d8bafc47f82f6efdba5a7ce3`.
- External Blender identity in the manifest and fresh profile: **4.5.14 LTS**, SHA-256 `57fa1d294ea76448c3bec84ca758caf4629611330abba7dcf55bb0c56a0a15ab`. It is not bundled.

The executable ran from `artifacts/packaged-smoke-FINAL-20261014-155450/unrelated working directory`, with a newly created application data directory containing only explicit external-tool paths. All other preferences started from defaults. The complete GUI smoke exited **0** in **35.837 seconds**. Its scene and preview revisions both ended at **11**, with **107,970 triangles** and `error:null`.

All **11 Boolean checks passed**:

- `cancel_received`
- `dismiss_unblocks_previous_model`
- `failed_reimport_preserves_settings`
- `latest_import_selected`
- `preview_display_intent_kept`
- `project_intent_supersedes_debounce`
- `project_roundtrip`
- `recovered_snapshot_keeps_job_identity`
- `recovery_blocks_new_exports`
- `saved_section_preserved`
- `stale_preview_rejected`

The additional `recovery_job` field is a retained job path, not a Boolean check. Evidence: `artifacts/packaged-smoke-FINAL-20261014-155450/verified-result.json`, `package-verification.json`, and `smoke/result.json`. Native/rendered UI screenshots and the final PNG/project remain alongside them. Visual inspection of `smoke/rendered-ui.png` confirmed the native model, authoritative rendered preview, physical height controls, absence of an error banner, and current-preview display intent.

All seven inventoried payload hashes were rechecked, and each of the ZIP's eight files (payload plus manifest) was compared byte-for-byte with the packaged files. Python generated a runtime `renderer/__pycache__` during the smoke; that cache is neither an intended payload nor present in the ZIP. Four actual retained render recipes (three complete, one cancelled) have worker-script, scene-script and Blender hashes matching the final package manifest. Source inspection of `renderer_script()` confirms the packaged scripts beside the executable take precedence; both exist at those expected paths. This is the resource-resolution evidence used here. CIM process command-line inspection remained access-denied and is explicitly not claimed as verified.

No app/model license was introduced, no external runtime executable was bundled, no source/user settings were replaced, and no build or application process remains active from this task. Root owns final full-tree checks and Conventional Commits.
