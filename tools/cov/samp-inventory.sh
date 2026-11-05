#!/usr/bin/env bash
# FEAT-SWEEP: function inventory of samp.dll (default: build B, the installed 2019 build) for
# the tracer's module coverage mode (SAMP_TRACER_COV_MOD) and the feature clustering.
#
#   tools/cov/samp-inventory.sh [SAMP_DLL]        # Ghidra (once) + export + arm list
#   COV_REANALYSE=1 tools/cov/samp-inventory.sh
#
# Own headless Ghidra project (target/ghidra-samp, never the REA project);
# tools/cov/ghidra/ExportGraph.java writes functions/edges/refs TSVs, then the arm list
# target/cov/samp-arm.txt holds `0xRVA BYTE` lines (entry byte of the unmodified file; only
# entries in the first executable section, not thunks). Generated data only.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DLL="${1:-$ROOT/re/samp-dll/0.3.7-R4_fv0.3.7.3_2019-11-21/samp.dll}"
PROJ="$ROOT/target/ghidra-samp"
OUT="$ROOT/target/cov"
PRE="$OUT/samp"
mkdir -p "$PROJ" "$OUT"
if [[ ! -s "$PRE.functions.tsv" || "${COV_REANALYSE:-0}" == 1 ]]; then
    if [[ -d "$PROJ/samp-cov.rep" && "${COV_REANALYSE:-0}" != 1 ]]; then
        /opt/ghidra/support/analyzeHeadless "$PROJ" samp-cov -process samp.dll -noanalysis -readOnly \
            -scriptPath "$ROOT/tools/cov/ghidra" -postScript ExportGraph.java "$PRE"
    else
        rm -rf "$PROJ/samp-cov.rep" "$PROJ/samp-cov.gpr"
        /opt/ghidra/support/analyzeHeadless "$PROJ" samp-cov -import "$DLL" -analysisTimeoutPerFile 3600 \
            -scriptPath "$ROOT/tools/cov/ghidra" -postScript ExportGraph.java "$PRE"
    fi
fi
python3 -I "$ROOT/tools/cov/samp_arm.py" "$PRE.functions.tsv" "$DLL" "$OUT/samp-arm.txt"
