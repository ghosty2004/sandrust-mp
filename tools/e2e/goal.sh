#!/usr/bin/env bash
# Combined goal scenario (phase 3 so far), one game session, lane "main":
#   game starts with our client → connects → class selection → spawns (Shift)
#   → the server probe sees our own movement (W held, state ONFOOT, >= 1 m)
#   → samp-bot appears as a remote player with a ped (slot >= 2), tracks the
#     server-side position and moves (control channel `remote`)
#   → the bot leaves and its ped is removed.
# It is tools/e2e/m3.sh with the walking step enabled (M3_WALK=1); screenshots
# target/goal-{walk,remote-1,remote-2}.png, logs target/goal/.
# Exit status 0 only if every step holds.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export M3_WALK=1 M3_SHOT=goal
exec "$ROOT/tools/e2e/m3.sh" "$@"
