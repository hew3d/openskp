#!/bin/sh
# Full clean-room verification: compile the Kaitai spec, parse the whole corpus
# with it, and semantically validate the reference parser against COLLADA.
set -e
cd "$(dirname "$0")/.."
PY="${PYTHON:-.venv/bin/python}"

echo "[1/4] compile ksy/skp.ksy -> tools/gen/skp.py"
node tools/compile_ksy.js ksy/skp.ksy tools/gen

echo "[2/4] Kaitai parse + header cross-check across corpus"
"$PY" tools/ksy_validate.py

echo "[3/4] semantic validation (vertices + materials) vs COLLADA ground truth"
"$PY" tools/validate.py

echo "[4/4] CArchive walker: class inventory + geometry pointer-graph resolution"
"$PY" tools/walk_validate.py

echo
echo "ALL GREEN"
