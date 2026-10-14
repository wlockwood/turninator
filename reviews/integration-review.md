# Integration review: initial CLI and proof controller

Reviewed `src/main.rs`, `tests/cli.rs`, `tools/prove-render.py`, and `tests/test_proof.py` against final-plan Tasks 1–3. No root-owned files changed. Findings below concern the inspected revision; root implementation continues concurrently.

## Findings to address

1. **High: percent sequences in a parent directory break GIF input.** `tools/prove-render.py:75` embeds the full arbitrary frame directory in an FFmpeg image-sequence pattern. Reproduced live with a temporary directory named `turninator-%04d-review-*`: two valid PNGs pass validation, but FFmpeg fails `No such file or directory` because the parent directory's `%04d` becomes another substitution. Invoke FFmpeg with `cwd=frames_dir` and only `frame_%04d.png` as its pattern; keep palette/output absolute. Add a real two-frame test under a directory containing `%04d`, spaces, Unicode and brackets. This is not shell escaping; argument arrays alone do not fix image-pattern interpretation.

2. **High: failed encoding cannot be retried through the runnable proof command.** `output.mkdir(exist_ok=True)` reuses the deterministic output directory, but the worker refuses the already-rendered `still.png` before reaching encoding. It also overwrites `blender.log` on the retry. A completed metrics file blocks all further use, while failed runs save no metrics/recipe record. Add an explicit encode-only mode (or verified reuse of complete frames) that loads a saved immutable recipe, validates frame dimensions/counts and matching render identities, then encodes without starting Blender. Persist recipe/probe/stage/error information before rendering, update it on failure, and preserve original logs. A full cross-session resume UI is deferred; usable encode retry is an initial foundation requirement.

3. **Medium: recipe fingerprint omits tool and encoder identities.** `tools/prove-render.py:247` hashes scene, mesh and worker scripts, but omits Blender binary/build, FFmpeg binary/build, proof/encoder script, palette/downsample recipe, and explicit start-angle/alpha choices. A tool upgrade or encoder change can collide with the existing proof path and recorded recipe. Record exact tool versions/hashes; include renderer identity in render fingerprint and encoder identity/options in a separate encoding fingerprint, allowing re-encoding without rerendering. Also compare worker-reported geometry hashes with the expected recipe hash so modifying job mesh files during a run cannot silently mix geometry across still/turntable requests.

4. **Medium: malformed protocol output is ignored until timeout.** `_read` creates a synthetic `error` event without `request_id` when prefixed JSON is malformed; `request` discards it at the ID filter. Treat malformed protocol lines as global transport failures before filtering request IDs. Validate that decoded event is an object with protocol version 1, known kind, matching revision and valid payload before accessing fields. Test malformed JSON, a valid JSON array, wrong protocol version and wrong-revision complete events using a fake worker; these should fail promptly and preserve the last valid result.

5. **Medium: request deadline is not enforced when queued events keep arriving.** `request` uses `max(0.1, deadline-now)` but never checks whether the deadline elapsed. A stream of unrelated/progress events can keep an expired request alive indefinitely. Explicitly check the monotonic deadline on each iteration and translate `queue.Empty` into an informative timeout containing request identity and log location. Make shutdown a short bounded wait rather than inheriting the 30-minute render limit. Test continuously arriving unrelated events and silence with an injected short timeout.

6. **Medium: accepted maximum image size differs between controller and worker.** Main accepts 8192 square; the worker accepts at most 8000 square, consistent with the 64-megapixel limit. A requested 8192 proof starts Blender before predictably failing. Use one 8000 maximum and reject before process startup. `validate_frames` can have an independent documented decode budget, but the CLI and render protocol must agree.

7. **Low: palette generation can read frames outside the validated selection.** Validation checks exactly the requested N files; palette generation then reads all consecutive files until EOF. If `frame_N.png` exists, it changes the palette despite not belonging to the requested animation. Limit the palette input to the selected N frames with an input trim/filter or a generated exact sequence. A regression test can add an extreme-color extra frame and verify that it is excluded.

## Lifecycle observations

The Windows Job Object has kill-on-close and parent process ownership is explicit. Cleanup targets the owned process; it does not enumerate other Blender processes. `finally` closes the owner after render/encode failures. These are good initial boundaries.

There is still a creation-to-assignment race: `Popen` starts Blender running before `AssignProcessToJobObject`. For the proof this is a narrowly bounded startup exposure; it does not establish the later desktop guarantee that all children are owned from inception. Before claiming that guarantee, create suspended, assign the Job Object, then resume using a supported process launcher. Also close the log if Popen itself fails (it currently opens before the guarded ownership block).

`run_encoder` uses `subprocess.run(timeout=600)`, which kills and waits for its direct child on timeout. It does not put FFmpeg in the same owned process tree. No evidence suggests this configured FFmpeg spawns children, so this is a documented scope limitation rather than a reason to block the rendering proof. Desktop integration should explicitly own encoder processes as planned.

`Worker.close` should be idempotent and tolerate a constructor or shutdown failure; reader/log handles should be closed after the reader exits. Current lifecycle tests cover encoding artifacts, not cancellation/restart or a live unrelated-process sentinel. Do not claim those paths verified until root runtime checks record them.

## CLI review

The CLI rejects unknown/repeated options, parses inputs and builds geometry before publishing a job, rejects existing output directories, writes scene JSON last through a temporary file, and preserves source bytes in an integration test. The generated fixture includes real physical dimensions and produces a distinct bead scene. No blocking CLI defect found in these reviewed files; parser and mesh correctness are reviewed separately.

An I/O failure after directory creation can leave mesh/temporary files but no published `scene.json`; this is acceptable as an incomplete job if documented. User-facing errors should direct the user to a fresh output directory or explicit cleanup, rather than promising transactional removal of all partial files.

## Existing verification strength and remaining tests

The GIF test genuinely runs installed FFmpeg, decodes every frame and checks 50 ms delays plus infinite looping. Destination preservation on a missing frame and dimension rejection are meaningful. Add encoder-process failure while an existing destination exists, corrupted PNG decoding, the percent-path case, and encode-only retry with no Blender launch. Existing good-input timing tests must continue checking the final frame delay.

The initial runtime proof should record both engines, two requests in one process, malformed-request recovery, real GIF output and 4K still. Separately exercise cancel/restart before asserting it works. Source checks and passing unit tests alone do not establish Windows lifetime behavior or visual bead quality.
