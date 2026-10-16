"""Qualify actual compiler output as well as validated owned record provenance."""
import hashlib
import json
from pathlib import Path
import struct
import sys

root = Path(sys.argv[1])
identity = json.loads((root / "runtime-identities.json").read_text(encoding="utf-8-sig"))
namespace = Path(identity["cache_directory"]) / "turninator-luxcore-ptx-v1" / identity["compiler_identity"] / identity["sdk_identity"]
records = []
for path in sorted(namespace.glob("*.ptxrec")):
    record = path.read_bytes()
    assert len(record) >= 272 and record[:8] == b"TNPTX01\0", "invalid record"
    compiler, sdk, key, digest = [record[i:i+64].decode("ascii") for i in (8, 72, 136, 200)]
    size = struct.unpack_from("<Q", record, 264)[0]
    ptx = record[272:]
    assert compiler == identity["compiler_identity"] and sdk == identity["sdk_identity"], "foreign identity"
    assert key == path.stem and size == len(ptx) and 0 < size <= 64 * 1024 * 1024 and ptx[-1] == 0, "invalid size/key"
    assert hashlib.sha256(ptx).hexdigest() == digest, "PTX digest mismatch"
    assert b"Cuda compilation tools, release 12.8," in ptx[:400], "actual compiler is not qualified 12.8"
    records.append(dict(key=key, ptx_sha256=digest, size=size,
                        record_sha256=hashlib.sha256(record).hexdigest(), mtime_ns=path.stat().st_mtime_ns,
                        compiler_header=ptx[:200].decode("ascii")))
assert len(records) >= 2, "expected OptiX and PathOCL records"
print(json.dumps(records, indent=2))
