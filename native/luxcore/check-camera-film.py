"""Actual alpha silhouette against the source wall's analytic camera projection.

Uses the shared frozen mask acceptance: IoU >= .995 and no disagreement beyond
one Chebyshev pixel. The wall is x=-2..2, y=0, z=4..8; camera looks along +Y.
"""
import json
import math
import pathlib
import struct
import sys
root = pathlib.Path(sys.argv[1])
reports = []
for alpha_path in sorted(root.glob('frame-*.alpha')):
    descriptor = alpha_path.with_suffix('.frame.json')
    f = json.loads(descriptor.read_text()) if descriptor.exists() else {'width':128, 'height':128, 'projection':1, 'vertical':5}
    width, height = f['width'], f['height']
    span = f['vertical'] if f['projection'] else 40 * math.tan(math.radians(f['vertical'] / 2))
    scale = height / span
    data = struct.unpack('<' + 'f' * (width * height * 2), alpha_path.read_bytes())
    actual = {(x, y) for y in range(height) for x in range(width)
              if data[(y * width + x) * 2 + 1] > 0 and data[(y * width + x) * 2] / data[(y * width + x) * 2 + 1] >= .5}
    expected = {(x, y) for y in range(height) for x in range(width)
                if abs(x + .5 - width / 2) < 2 * scale and abs(y + .5 - height / 2) < 2 * scale}
    def dilate(mask):
        return {(x+dx,y+dy) for x,y in mask for dx in (-1,0,1) for dy in (-1,0,1)}
    iou = len(actual & expected) / len(actual | expected)
    outside = len(actual - dilate(expected)) + len(expected - dilate(actual))
    reports.append({'film': alpha_path.name, 'width': width, 'height': height,
                    'projection': f['projection'], 'iou': iou, 'outside_one_pixel': outside})
(root/'camera-mask-comparison.json').write_text(json.dumps(reports,indent=2))
print(json.dumps(reports,indent=2))
assert reports
assert all(r['iou'] >= .995 and r['outside_one_pixel'] == 0 for r in reports), 'physical camera source silhouette mismatch'
