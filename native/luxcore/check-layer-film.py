"""Compare the actual direct wall film to the shared physical-normal intent.

This is an independent test oracle, transcribed from geometry.rs's documented
analytic normal, integrated over a pixel rather than mirroring the GPU band
implementation. The fixture uses an orthographic 5 mm span and source Z=4..8.
"""
import json
import math
import pathlib
import struct
import sys

root = pathlib.Path(sys.argv[1])
raw = (root / "frame-3.normal").read_bytes()
values = struct.unpack("<" + "f" * (len(raw) // 4), raw)
assert len(values) == 128 * 128 * 3
assert all(math.isfinite(v) for v in values)
rows = []
for y in range(14, 114):
    expected = [0.0, 0.0, 0.0]
    for sample in range(256):
        z = -.5 + (y + (sample + .5) / 256) * 5 / 128
        pitch = .6 if z < .6 else .3
        phase = z / .6 if z < .6 else (z - .6) / .3
        derivative = math.pi * .06 / pitch * math.sin(2 * math.pi * phase)
        length = math.sqrt(1 + derivative * derivative)
        expected[1] += -1 / length / 256
        expected[2] += -derivative / length / 256
    actual = [sum(values[(y * 128 + x) * 3 + k] for x in range(20, 108)) / 88 for k in range(3)]
    rows.append({"row": y, "build_z_mm": -.5 + (y + .5) * 5 / 128,
                 "expected_normal": expected, "actual_normal": actual,
                 "maximum_component_error": max(abs(a - e) for a, e in zip(actual, expected))})
errors = [r["maximum_component_error"] for r in rows]
report = {"oracle": "geometry-layer-normal-pixel-integral-v1",
          "graph": "physical-tangent-band16-linear-v1",
          "analytic_sine_interpolation_bound": (2 * math.pi / 15) ** 2 / 8,
          "maximum_row_normal_error": max(errors),
          "rms_row_normal_error": math.sqrt(sum(e * e for e in errors) / len(errors)),
          "first_layer_max_error": max(r["maximum_component_error"] for r in rows if r["build_z_mm"] < .6),
          "later_layer_max_error": max(r["maximum_component_error"] for r in rows if r["build_z_mm"] >= .6),
          "rows": rows}
(root / "normal-comparison.json").write_text(json.dumps(report, indent=2))
print(json.dumps({k: v for k, v in report.items() if k != "rows"}, indent=2))
# 88 pixels x >=64 actual samples/row. These account for stochastic subpixel
# integration, not geometry-boundary tolerance or pitch changes.
assert report["maximum_row_normal_error"] < .05, "actual GPU normals differ from physical layer intent"
assert report["rms_row_normal_error"] < .02, "actual GPU normal error is systematic"
