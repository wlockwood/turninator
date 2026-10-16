"""Compare a compiled port/reference with the frozen, SDK-generated color chart.

python assets/render/color/compare.py path/to/color_port.exe
Add --freeze-reference only with the independently compiled pinned SDK reference.
This tool neither builds nor downloads anything.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("executable", type=Path)
parser.add_argument("--freeze-reference", action="store_true")
args = parser.parse_args()
if args.freeze_reference:
    identity = subprocess.run([str(args.executable.resolve()), "--identity"],
                              capture_output=True, text=True, check=True).stdout.strip()
    assert identity == "filament-agx-reference-v1", "freeze requires the pinned SDK reference"
chart_path = Path(__file__).resolve().parents[3] / "tests/fixtures/renderers/color-chart.json"
chart = json.loads(chart_path.read_text(encoding="utf-8"))
inputs = "".join(" ".join(map(str, row["scene_rgba"] + [row["exposure_ev"]])) + "\n"
                 for row in chart["patches"])
result = subprocess.run([str(args.executable.resolve())], input=inputs, text=True,
                        capture_output=True, check=True)
lines = result.stdout.splitlines()
assert len(lines) == len(chart["patches"]), (len(lines), len(chart["patches"]))
worst = 0.0
for row, line in zip(chart["patches"], lines):
    values = list(map(float, line.split()))
    assert len(values) == 8 and all(map(math.isfinite, values)), row["name"]
    for field, actual in [("display_linear_rgba", values[:4]), ("display_srgb_rgba", values[4:])]:
        if args.freeze_reference:
            row[field] = actual
        else:
            error = max(abs(a - b) for a, b in zip(actual, row[field]))
            worst = max(worst, error)
            assert error <= chart["absolute_tolerance"], (row["name"], field, error)
if args.freeze_reference:
    chart_path.write_text(json.dumps(chart, indent=2) + "\n", encoding="utf-8")
    print(f"froze {len(lines)} SDK reference patches")
else:
    print(f"PASS {len(lines)} patches x linear/sRGB RGBA; max absolute error {worst:.9g}")
