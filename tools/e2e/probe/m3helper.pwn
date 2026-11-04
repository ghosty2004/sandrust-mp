// M3 e2e helper filterscript: brings the test bot next to the in-game client
// so the server streams it in. Loaded next to e2eprobe by tools/e2e/m3.sh.
// When a player named "RemoteBot*" spawns, 1.5 s later (after the gamemode's
// own spawn placement) it is put 5 m in front of the first spawned non-bot
// player, facing it. Logs "[m3] bring id=… near=… pos=x,y,z".
#define FILTERSCRIPT
#include <open.mp>

forward M3Bring(botid);

static bool:IsTestBot(playerid)
{
    new name[MAX_PLAYER_NAME + 1];
    GetPlayerName(playerid, name, sizeof name);
    return strfind(name, "RemoteBot") == 0;
}

public OnFilterScriptInit() { print("[m3] helper loaded"); return 1; }

public OnPlayerSpawn(playerid)
{
    if (IsTestBot(playerid)) SetTimerEx("M3Bring", 1500, false, "i", playerid);
    return 1;
}

public M3Bring(botid)
{
    if (!IsPlayerConnected(botid)) return 0;
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (i == botid || !IsPlayerConnected(i) || IsTestBot(i)) continue;
        if (GetPlayerState(i) != PLAYER_STATE_ONFOOT) continue;
        new Float:x, Float:y, Float:z, Float:a;
        GetPlayerPos(i, x, y, z);
        GetPlayerFacingAngle(i, a);
        new Float:bx = x - 5.0 * floatsin(a, degrees), Float:by = y + 5.0 * floatcos(a, degrees);
        SetPlayerInterior(botid, GetPlayerInterior(i));
        SetPlayerVirtualWorld(botid, GetPlayerVirtualWorld(i));
        SetPlayerPos(botid, bx, by, z);
        SetPlayerFacingAngle(botid, a + 180.0);
        printf("[m3] bring id=%d near=%d pos=%.2f,%.2f,%.2f", botid, i, bx, by, z);
        return 1;
    }
    SetTimerEx("M3Bring", 1000, false, "i", botid);
    return 1;
}
