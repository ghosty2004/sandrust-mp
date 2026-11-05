#!/usr/bin/env bash
# F6 proof (docs/PLAN.md): data parity — gta_sa.exe gets the same files from the user's samp.saa
# under our client as under the original samp.dll (decision D-4 = A).
#  1. holds the trace-lane lock (tools/trace/with-lock.sh) for both runs, so nothing else uses
#     wine/prefix-trace, wine/gta-sa-trace, UDP 7790 or control ports 47701/47702 meanwhile
#  2. tools/trace/run.sh orig spawn, then tools/trace/run.sh ours spawn (boot → join → class
#     selection → Shift → spawn → walk → quit on the local test server), both with
#     SAMP_TRACER_FILEIO=1: samp-tracer wraps gta_sa.exe's CreateFileA / ReadFile / GetFileSize /
#     SetFilePointer / CloseHandle import slots at WinMain (on top of the client's redirection)
#     and records every open (path, archive handle vs disk, access), the size the client reports
#     and a SHA-1 of the bytes the game actually read (crates/samp-tracer/src/win/fileio.rs)
#  3. tools/diff/files_report.py compares: the multiset and the order of opens, the order of the
#     opens served from samp.saa, sizes, content hashes (every archive-served open, plus read-only
#     disk files), and our client's own log (`files: CreateFileA(...) -> archive handle`, entry
#     sizes) against our tracer records
# Only the lane's game copy is used (wine/gta-sa-trace/samp.saa); nothing is written to the user's
# SA:MP or GTA install; only digests are printed, never archive content.
# Exit 0 = identical, or every residual is accepted in tools/diff/files-decisions.tsv by a decision
# logged in docs/ROADMAP.md "Decisions made autonomously".
# Usage: tools/diff/files.sh                      run both clients, then report
#        tools/diff/files.sh --report ORIG OURS   report on two existing trace output dirs
# Output: target/files/<stamp>/ (run logs, summary.txt; trace dirs stay in target/trace/).
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DATA_ROOT="${SAMP_RS_ROOT:-$ROOT}"
say() { printf '[files] %s\n' "$*"; }

if [[ "${1:-}" == --report ]]; then
    [[ $# -eq 3 ]] || { echo "usage: $0 --report ORIG_DIR OURS_DIR" >&2; exit 2; }
    exec python3 -I "$ROOT/tools/diff/files_report.py" "$2" "$3"
fi

# ---- inner part: runs while holding the trace-lane lock --------------------------------------
if [[ "${1:-}" == --inner ]]; then
    OUT="$2"
    export SAMP_TRACER_FILEIO=1
    for client in orig ours; do
        say "running $client (tools/trace/run.sh $client spawn)"
        "$ROOT/tools/trace/run.sh" "$client" spawn > "$OUT/$client.out" 2>&1
        rc=$?
        dir="$(sed -nE 's/^\[trace\] output: ([^ ]+) .*/\1/p' "$OUT/$client.out" | tail -n 1)"
        say "  $client: exit $rc, output ${dir:-?}"
        if (( rc != 0 )) || [[ -z "$dir" || ! -f "$dir/trace.jsonl" ]]; then
            tail -n 15 "$OUT/$client.out" | sed 's/^/    /'
            say "FAIL: the $client run did not complete"
            exit 1
        fi
        echo "$dir" > "$OUT/$client.dir"
    done
    exit 0
fi

# ---- outer part ------------------------------------------------------------------------------
[[ -f "$DATA_ROOT/wine/gta-sa-trace/samp.saa" ]] \
    || { say "no samp.saa in wine/gta-sa-trace (the trace lane's game copy)"; exit 1; }
[[ -f "$DATA_ROOT/data/auth_table.txt" ]] || { say "data/auth_table.txt missing (tools/extract_auth_table.py)"; exit 1; }
OUT="$ROOT/target/files/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
say "output: $OUT"
"$ROOT/tools/trace/with-lock.sh" bash "$0" --inner "$OUT" || exit 1
python3 -I "$ROOT/tools/diff/files_report.py" "$(cat "$OUT/orig.dir")" "$(cat "$OUT/ours.dir")" \
    --summary "$OUT/summary.txt"
rc=$?
say "$( ((rc == 0)) && echo PASS || echo FAIL ) (summary: $OUT/summary.txt)"
exit "$rc"
