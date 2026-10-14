"""Live owned cancellation/restart and corrupted-frame recovery checks."""
import argparse
import importlib.util
import json
import pathlib
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("proof", ROOT / "tools" / "prove-render.py")
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job", type=pathlib.Path, required=True)
    parser.add_argument("--blender", default="C:/Program Files/Blender Foundation/Blender 4.5/blender.exe")
    args = parser.parse_args()
    job = args.job.resolve()
    output = job / ("runtime-" + uuid.uuid4().hex[:10])
    output.mkdir()
    workers = []
    result = {"scope": "cancel during preparation, unrelated worker preservation, restart, corrupt frame, verified reuse"}
    try:
        sentinel = proof.Worker(args.blender, output / "sentinel.log")
        workers.append(sentinel)
        sentinel.request("probe", "sentinel-before")
        victim = proof.Worker(args.blender, output / "cancelled.log")
        workers.append(victim)
        victim.request("probe", "victim-probe")
        victim.process.stdin.write(json.dumps({"protocol_version": 1, "request_id": "cancel-me",
            "scene_revision": 1, "kind": "render", "scene_path": str(job / "scene.json"),
            "output_dir": str(output / "cancelled-frames"), "mode": "turntable", "engine": "eevee",
            "size": 64, "samples": 8, "frame_count": 1000, "start_angle_degrees": 0,
            "transparent": False}) + "\n")
        victim.process.stdin.flush()
        time.sleep(0.4)
        cancelled_pid = victim.process.pid
        victim.close()
        if victim.process.poll() is None:
            raise RuntimeError("Owned worker remained alive after cancellation")
        sentinel.request("probe", "sentinel-after")
        if sentinel.process.poll() is not None:
            raise RuntimeError("Cancellation affected unrelated worker")
        result.update({"cancelled_pid": cancelled_pid, "cancelled_returncode": victim.process.returncode,
                       "sentinel_pid": sentinel.process.pid, "sentinel_preserved": True})
        replacement = proof.Worker(args.blender, output / "restarted.log")
        workers.append(replacement)
        replacement.request("probe", "restart-probe")
        options = {"scene_path": str(job / "scene.json"), "output_dir": str(output / "restart-still"),
                   "mode": "still", "engine": "eevee", "size": 64, "samples": 8,
                   "frame_count": 1, "start_angle_degrees": 0, "transparent": False}
        replacement.request("render", "restart-render", **options)
        image = output / "restart-still" / "still.png"
        image.write_bytes(b"damaged PNG fixture")
        repaired = replacement.request("render", "repair-render", **options, reuse_verified_frames=True)
        if next(e for e in repaired if e["kind"] == "frame")["payload"]["reused"]:
            raise RuntimeError("Corrupted frame was incorrectly reused")
        with proof.Image.open(image) as decoded:
            decoded.load()
            if decoded.size != (64, 64):
                raise RuntimeError("Repair did not produce valid PNG")
        reused = replacement.request("render", "verified-reuse", **options, reuse_verified_frames=True)
        if not next(e for e in reused if e["kind"] == "frame")["payload"]["reused"]:
            raise RuntimeError("Valid matching frame was not reused")
        result.update({"restart_rendered": True, "corrupt_frame_rerendered": True, "matching_frame_reused": True})
        for worker in (replacement, sentinel):
            worker.request("shutdown", "runtime-shutdown")
            worker.process.wait(timeout=20)
            if worker.process.returncode:
                raise RuntimeError("Runtime worker shutdown failed")
        proof.write_json(output / "result.json", result)
        print(json.dumps({"result": str(output / "result.json"), **result}, indent=2))
    finally:
        for worker in reversed(workers):
            worker.close()


if __name__ == "__main__":
    main()
