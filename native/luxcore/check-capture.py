"""Check actual SDK mesh captures; no GPU or implementation-formula oracle."""
import json
import pathlib
import struct
import sys

root = pathlib.Path(sys.argv[1])
reports = []
for source in sorted(root.glob("upload-*.json")):
    data = json.loads(source.read_text())
    mesh = source.with_name(source.stem + "-mesh0.ply").read_bytes()
    header, binary = mesh.split(b"end_header\n", 1)
    fields = header.decode("ascii").splitlines()
    vertex_properties = fields[fields.index("element vertex 4") + 1:fields.index("element face 2")]
    expected = ["property float " + name for name in ("x", "y", "z", "nx", "ny", "nz")]
    if vertex_properties != expected:
        raise AssertionError(f"{source.name}: SDK introduced absent attributes: {vertex_properties}")
    vertices = [struct.unpack_from("<6f", binary, i * 24) for i in range(4)]
    for i, row in enumerate(vertices):
        assert row[:3] == tuple(data["positions"][i * 3:i * 3 + 3]), "SDK position changed"
        assert row[3:] == tuple(data["normals"][i * 3:i * 3 + 3]), "SDK normal changed"
    faces = struct.unpack_from("<B3IB3I", binary, 96)
    assert faces == (3, 0, 1, 2, 3, 0, 2, 3), "SDK face indices changed"
    reports.append({"source": source.name, "vertices": 4, "triangles": 2, "unexpected_attributes": 0})
assert reports, "no actual SDK mesh captures"
print(json.dumps({"sdk_mesh_capture_pass": reports}, indent=2))
