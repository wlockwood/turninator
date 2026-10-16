"""Make patched text byte-identical across Git autocrlf configurations."""
from pathlib import Path
import sys

source, patch = Path(sys.argv[1]), Path(sys.argv[2])
for line in patch.read_text().splitlines():
    if line.startswith("+++ b/"):
        file = source / line[6:]
        file.write_text(file.read_text(), newline="\n")
