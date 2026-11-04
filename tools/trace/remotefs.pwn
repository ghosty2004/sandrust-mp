// TRACE-2 filterscript (trace lane only): when the player named "RemoteBot"
// spawns, teleport it 3 m east of the first other spawned player, so it is
// inside that player's streaming range; 15 s later move it 1500 m away
// (stream-out), 10 s after that bring it back (stream-in again, slot reuse).
// Logs "[trace-fs] ..." lines.
#define FILTERSCRIPT
#include <open.mp>

public OnFilterScriptInit() { print("[trace-fs] loaded"); return 1; }

public OnPlayerSpawn(playerid)
{
    new name[MAX_PLAYER_NAME + 1]; GetPlayerName(playerid, name, sizeof name);
    if (!strcmp(name, "RemoteBot"))
    {
        SetTimerEx("TraceBringBot", 500, false, "i", playerid);
        SetTimerEx("TraceSendBotAway", 15500, false, "i", playerid);
        SetTimerEx("TraceBringBot", 25500, false, "i", playerid);
    }
    return 1;
}

forward TraceSendBotAway(botid);
public TraceSendBotAway(botid)
{
    if (!IsPlayerConnected(botid)) return 1;
    new Float:x, Float:y, Float:z;
    GetPlayerPos(botid, x, y, z);
    SetPlayerPos(botid, x + 1500.0, y, z);
    printf("[trace-fs] bot %d sent away to %.2f,%.2f,%.2f", botid, x + 1500.0, y, z);
    return 1;
}

forward TraceBringBot(botid);
public TraceBringBot(botid)
{
    if (!IsPlayerConnected(botid)) return 1;
    new Float:x, Float:y, Float:z;
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (i == botid || !IsPlayerConnected(i) || GetPlayerState(i) != PLAYER_STATE_ONFOOT) continue;
        GetPlayerPos(i, x, y, z);
        SetPlayerInterior(botid, GetPlayerInterior(i));
        SetPlayerVirtualWorld(botid, GetPlayerVirtualWorld(i));
        SetPlayerPos(botid, x + 3.0, y, z);
        printf("[trace-fs] bot %d moved next to %d at %.2f,%.2f,%.2f", botid, i, x + 3.0, y, z);
        return 1;
    }
    print("[trace-fs] no spawned player to move the bot to");
    return 1;
}
