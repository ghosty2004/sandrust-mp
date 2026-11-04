#!/usr/bin/env bash
# TRACE-6: material text render capture and object edit/select flows in a running hold session.
#   1) d3d `ui` log while an object with material text is created (render target + DrawText layout);
#   2) EditObject → screenshot → ESC (117 response 0); EditAttachedObject → ESC (116);
#   3) SelectObject → click the object in the middle of the screen (27) → CancelEdit.
set -uo pipefail
U="$(cd "$(dirname "$0")" && pwd)/ui.sh"
mark() { "$U" mark "$1 wall=$(date +%s.%N)" > /dev/null; }
"$U" fs 'pos|L|1700|-2600|13.55|0' > /dev/null
sleep 2
"$U" d3d-start ui 120 mattext
mark mattext
"$U" fs 'obj|19353|1700|-2594|14.5|0|0|0|0'
"$U" fs 'objtext|1|0|RE10 {FF0000}red|130|Arial|24|1|FFFFFFFF|FF000000|1'
"$U" d3d-collect mattext
"$U" shot w-mattext > /dev/null
"$U" fs 'obj|1271|1700|-2596|13.9|0|0|0|0'
sleep 2
mark objedit
"$U" fs 'objedit|L|2' > /dev/null
sleep 2
"$U" shot w-objedit > /dev/null
"$U" d3d ui 3 objedit > /dev/null
"$U" key Escape
sleep 1.5
mark attedit
"$U" fs 'att|L|0|18645|2|0.1|0|0|0|90|0|1|1|1' > /dev/null
sleep 1
"$U" fs 'attedit|L|0' > /dev/null
sleep 2
"$U" shot w-attedit > /dev/null
"$U" key Escape
sleep 1.5
mark objsel
"$U" fs 'objsel|L' > /dev/null
sleep 2
"$U" shot w-objsel > /dev/null
"$U" mouse mousemove 640 330
sleep 0.5
"$U" shot w-objsel-hover > /dev/null
"$U" mouse click 1
sleep 1.5
"$U" fs 'editcancel|L' > /dev/null
mark world2done
"$U" log 40 | grep -E "edit|select" | cut -c30-
