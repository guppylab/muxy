#!/usr/bin/env python3
"""Audit a native Linux CLI's architecture, system linkage, and glibc floor."""

import argparse
import os
import re
import subprocess
import sys

ALLOWED_LIBRARIES = {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2", "librt.so.1", "libutil.so.1"}
TARGETS = {
    "x86_64-unknown-linux-gnu": ("Advanced Micro Devices X86-64", "/lib64/ld-linux-x86-64.so.2"),
    "aarch64-unknown-linux-gnu": ("AArch64", "/lib/ld-linux-aarch64.so.1"),
}


def audit(header, segments, dynamic, versions, linked, target):
    machine, interpreter = TARGETS[target]
    if not re.search(r"Class:\s+ELF64", header) or not re.search(r"Data:.*little endian", header) or not re.search(r"Machine:\s+" + re.escape(machine) + r"\s*(?:\n|$)", header):
        raise ValueError("ELF architecture does not match the requested target")
    if f"[Requesting program interpreter: {interpreter}]" not in segments:
        raise ValueError("Unexpected or missing glibc interpreter")
    libraries = set(re.findall(r"\(NEEDED\).*\[([^]]+)\]", dynamic))
    if "libc.so.6" not in libraries or libraries - ALLOWED_LIBRARIES:
        raise ValueError(f"Unexpected ELF dependencies: {sorted(libraries)}")
    if re.search(r"\((?:RPATH|RUNPATH)\)", dynamic):
        raise ValueError("CLI must not contain a build-path RPATH/RUNPATH")
    requirements = set(re.findall(r"Name: (GLIBC_[^\s]+)", versions))
    if not requirements:
        raise ValueError("Missing GLIBC version requirements")
    for requirement in requirements:
        version = requirement.removeprefix("GLIBC_")
        if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)+", version) or tuple(map(int, version.split("."))) > (2, 35):
            raise ValueError(f"CLI requires {requirement}, above the glibc 2.35 floor")
    if "not found" in linked:
        raise ValueError("ELF dependency is unavailable")
    for path in re.findall(r"(?:=>\s*)?(/[^\s]+)", linked):
        if not path.startswith(("/lib/", "/lib64/", "/usr/lib/", "/usr/lib64/")):
            raise ValueError(f"Non-system runtime library: {path}")
    return sorted(libraries), sorted(requirements)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("--target", choices=TARGETS, required=True)
    args = parser.parse_args()
    env = {k: v for k, v in os.environ.items() if k not in {"LD_PRELOAD", "LD_LIBRARY_PATH"}}
    env["LC_ALL"] = "C"
    def output(*command):
        return subprocess.check_output(command, text=True, env=env)
    libraries, versions = audit(output("readelf", "-hW", args.binary), output("readelf", "-lW", args.binary),
        output("readelf", "-dW", args.binary), output("readelf", "-VW", args.binary), output("ldd", args.binary), args.target)
    print(f"Verified {args.target}: {', '.join(libraries)}; GLIBC requirements: {', '.join(versions)}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
