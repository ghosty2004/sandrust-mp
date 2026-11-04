#!/usr/bin/env bash
# COV-1 step 1: function inventory of gta_sa.exe 1.0 US.
#
#   tools/cov/inventory.sh            # Ghidra (if needed) + names → target/cov/gta-functions.tsv
#   COV_REANALYSE=1 tools/cov/inventory.sh
#
# Runs its own headless Ghidra (project target/ghidra-cov, never the REA project), exports
# every function entry with tools/cov/ghidra/ExportFunctions.java, then tools/cov/names.py
# adds names from re/gta-reversed and re/plugin-sdk and the arming verdict for the tracer's
# coverage mode. Generated data only (target/ is gitignored).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXE="$ROOT/re/gta/1.0-US/gta_sa.exe"
PROJ="$ROOT/target/ghidra-cov"
OUT="$ROOT/target/cov"
RAW="$OUT/ghidra-functions.tsv"
mkdir -p "$PROJ" "$OUT"

if [[ ! -s "$RAW" || "${COV_REANALYSE:-0}" == 1 ]]; then
    if [[ -d "$PROJ/gta-cov.rep" && "${COV_REANALYSE:-0}" != 1 ]]; then
        # Reuse the analysed project; only run the export script.
        GHIDRA_HEADLESS_MAXMEM="${GHIDRA_HEADLESS_MAXMEM:-6G}" /opt/ghidra/support/analyzeHeadless \
            "$PROJ" gta-cov -process gta_sa.exe -noanalysis -readOnly \
            -scriptPath "$ROOT/tools/cov/ghidra" -postScript ExportFunctions.java "$RAW"
    else
        rm -rf "$PROJ/gta-cov.rep" "$PROJ/gta-cov.gpr"
        GHIDRA_HEADLESS_MAXMEM="${GHIDRA_HEADLESS_MAXMEM:-6G}" /opt/ghidra/support/analyzeHeadless \
            "$PROJ" gta-cov -import "$EXE" -analysisTimeoutPerFile 7200 \
            -scriptPath "$ROOT/tools/cov/ghidra" -postScript ExportFunctions.java "$RAW"
    fi
fi
python3 "$ROOT/tools/cov/names.py" "$RAW" "$EXE" "$OUT/gta-functions.tsv"
