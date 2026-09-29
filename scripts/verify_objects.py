#!/usr/bin/env python3
"""Compare complete C object files, using the original parser's include directory."""
import argparse
from pathlib import Path
import subprocess
import tempfile

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("original", type=Path)
p.add_argument("minified", type=Path)
p.add_argument("--cc", default="cc")
p.add_argument("--cpp-arg", action="append", default=[])
args = p.parse_args()

with tempfile.TemporaryDirectory(prefix="tree-trimmer-objects-") as directory:
    objects = []
    for index, source in enumerate((args.original, args.minified)):
        output = Path(directory) / f"{index}.o"
        subprocess.run(
            [args.cc, "-x", "c", "-std=c11", "-g0", "-iquote",
             str(args.original.resolve().parent), *args.cpp_arg,
             "-c", "-", "-o", str(output)],
            input=source.read_bytes(), check=True,
        )
        objects.append(output.read_bytes())
    if objects[0] != objects[1]:
        raise SystemExit("Object files differ (debug/compiler metadata can also cause differences).")
    print(f"{args.cc}: byte-identical object files ({len(objects[0]):,} bytes)")
