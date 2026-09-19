"""Prepare an exact-build development installer; never installs or requests sudo."""
import hashlib
from pathlib import Path
import sys

build = Path(sys.argv[1]).resolve()
template = Path(__file__).with_name("install-development.py.in").read_text()
template = template.replace("BUILD_SHA256", hashlib.sha256((build / "ExtendComputerLowJitter").read_bytes()).hexdigest())
(build / "install.py").write_text(template)
