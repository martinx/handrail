#!/usr/bin/env python3
"""Render the Homebrew formula from a release's version and SHA256SUMS.

    python3 packaging/homebrew/render.py 0.1.1 SHA256SUMS > handrail.rb
"""
import re
import sys
from pathlib import Path

version, sums = sys.argv[1], Path(sys.argv[2]).read_text()
text = (Path(__file__).parent / "handrail.rb.in").read_text().replace("@VERSION@", version)
for line in sums.splitlines():
    digest, name = line.split()
    m = re.fullmatch(r"\*?handrail-(.+)\.tar\.gz", name)
    if m:
        text = text.replace(f"@SHA_{m.group(1)}@", digest)
left = re.findall(r"@[A-Za-z0-9_-]+@", text)
if left:
    sys.exit(f"missing checksums for: {left}")
sys.stdout.write(text)
