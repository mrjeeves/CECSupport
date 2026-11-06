#!/usr/bin/env python3
"""Gate public Linux downloads on the AllMyStuff release with Wayland fixes."""
import os
from pathlib import Path
import re

MINIMUM = (0, 2, 123)
root = Path(__file__).resolve().parent.parent
pin = (root / ".allmystuff-rev").read_text().strip()
match = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", pin)
ready = bool(match and tuple(map(int, match.groups())) >= MINIMUM)
if output := os.environ.get("GITHUB_OUTPUT"):
    with open(output, "a", encoding="utf-8") as stream:
        stream.write(f"ready={str(ready).lower()}\n")
if ready:
    print(f"Linux publication enabled: AllMyStuff {pin} meets v0.2.123.")
else:
    print(f"::notice::Linux candidates only: AllMyStuff {pin} must advance to v0.2.123 or newer, with a matching committed Cargo lock, before public Linux downloads are uploaded.")
