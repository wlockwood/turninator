"""Extract only regular members from the pinned source into a new owned directory."""
import hashlib
from pathlib import Path, PurePosixPath
import sys
import tarfile

archive, target = Path(sys.argv[1]), Path(sys.argv[2]).resolve()
expected = "31a82d87c1f912864e7b039620747507d2b0ce9db70648656d41d0ebc4636cda"
assert hashlib.sha256(archive.read_bytes()).hexdigest() == expected, "source archive identity differs"
assert not target.exists(), "source output must be fresh"
target.mkdir(parents=True)
with tarfile.open(archive) as source:
    for member in source:
        parts = PurePosixPath(member.name).parts
        assert parts[0] == "LuxCore-b872186d2fe995a762f73708c606847eed55843e"
        if len(parts) == 1:
            continue
        relative = Path(*parts[1:])
        destination = (target / relative).resolve()
        assert destination.is_relative_to(target), "archive path escape"
        if member.isdir():
            destination.mkdir(parents=True, exist_ok=True)
        elif member.isfile():
            destination.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as stream:
                destination.write_bytes(stream.read())
        else:
            # The source contains an irrelevant dangling editor symlink. No
            # build consumes it, and we never materialize archive links.
            print("Skipped non-regular archive member:", member.name)
