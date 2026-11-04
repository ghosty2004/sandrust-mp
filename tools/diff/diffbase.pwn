// DIFF-2 base gamemode (diff lane only): one class, fixed spawn, no timers and
// no randomness, so the original client and ours see exactly the same server.
// The RPC steps themselves live in tools/diff/rpcfire.pwn (filterscript).
#include <open.mp>

#define DIFF_SPAWN_X   1958.3783
#define DIFF_SPAWN_Y   1343.1572
#define DIFF_SPAWN_Z   15.3746
#define DIFF_SPAWN_A   270.0

main() {}

public OnGameModeInit()
{
    SetGameModeText("diffbase");
    // Skin 60, no weapons; the same class for players and open.mp NPCs.
    AddPlayerClass(60, DIFF_SPAWN_X, DIFF_SPAWN_Y, DIFF_SPAWN_Z, DIFF_SPAWN_A);
    print("[diff] base gamemode loaded");
    return 1;
}

public OnPlayerRequestClass(playerid, classid)
{
    SetPlayerPos(playerid, DIFF_SPAWN_X, DIFF_SPAWN_Y, DIFF_SPAWN_Z);
    SetPlayerFacingAngle(playerid, DIFF_SPAWN_A);
    SetPlayerCameraPos(playerid, DIFF_SPAWN_X + 6.0, DIFF_SPAWN_Y, DIFF_SPAWN_Z + 1.5);
    SetPlayerCameraLookAt(playerid, DIFF_SPAWN_X, DIFF_SPAWN_Y, DIFF_SPAWN_Z + 0.5);
    return 1;
}

public OnPlayerSpawn(playerid) { return 1; }
