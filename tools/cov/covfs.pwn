// COV-1 filterscript (lanes diff/diff2 only): moves the player between the coverage
// scenario's places, driven by tools/cov/drive.py over RCON. Every command ends with " ;"
// (RCON text from the query port carries stale bytes after its length).
//   cov grove <id> ;   Grove Street (2495, -1675), facing south
//   cov enex <id> ;    outside the Alhambra entry-exit ("BAR1"), facing its marker (east)
//   cov car <id> ;     back to Grove Street with a Sultan next to the player, driver door
//                      towards it (player faces east)
//   cov info <id> ;    [cov] info id=.. st=.. int=.. vw=.. veh=.. pos=x,y,z
// Every player gets its own virtual world at spawn (id + 1), so two clients on this server
// never stream each other. World bounds are lifted. Logs "[cov] ..." lines.
#define FILTERSCRIPT
#include <open.mp>

new gCar[MAX_PLAYERS] = { INVALID_VEHICLE_ID, ... };

public OnFilterScriptInit() { print("[cov] covfs loaded"); return 1; }

public OnPlayerSpawn(playerid)
{
    SetPlayerVirtualWorld(playerid, playerid + 1);
    SetPlayerWorldBounds(playerid, 20000.0, -20000.0, 20000.0, -20000.0);
    printf("[cov] spawned id=%d vw=%d", playerid, playerid + 1);
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    if (gCar[playerid] != INVALID_VEHICLE_ID) { DestroyVehicle(gCar[playerid]); gCar[playerid] = INVALID_VEHICLE_ID; }
    return 1;
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[cov] state id=%d %d->%d veh=%d", playerid, _:oldstate, _:newstate, GetPlayerVehicleID(playerid));
    return 1;
}

public OnPlayerInteriorChange(playerid, newinteriorid, oldinteriorid)
{
    printf("[cov] interior id=%d %d->%d", playerid, oldinteriorid, newinteriorid);
    return 1;
}

stock CovTokens(const cmd[], tok[][], maxtok, toklen)
{
    new n = 0, i = 0, k = 0;
    while (n < maxtok)
    {
        while (cmd[i] == ' ') i++;
        if (cmd[i] == '\0' || cmd[i] == ';') break;
        k = 0;
        while (cmd[i] != '\0' && cmd[i] != ' ' && cmd[i] != ';')
        {
            if (k < toklen - 1) { tok[n][k] = cmd[i]; k++; }
            i++;
        }
        tok[n][k] = '\0';
        n++;
    }
    return n;
}

stock CovPlace(id, Float:x, Float:y, Float:z, Float:a)
{
    SetPlayerWorldBounds(id, 20000.0, -20000.0, 20000.0, -20000.0);
    SetPlayerInterior(id, 0);
    SetPlayerPos(id, x, y, z);
    SetPlayerFacingAngle(id, a);
    SetCameraBehindPlayer(id);
}

public OnRconCommand(cmd[])
{
    if (strcmp(cmd, "cov ", false, 4)) return 0;
    new tok[4][24];
    new n = CovTokens(cmd[4], tok, sizeof tok, sizeof tok[]);
    if (n < 2) { print("[cov] error needs a command and a player id"); return 1; }
    new id = strval(tok[1]);
    if (!IsPlayerConnected(id)) { printf("[cov] error %s: player %d not connected", tok[0], id); return 1; }
    if (!strcmp(tok[0], "grove"))
    {
        CovPlace(id, 2495.0, -1675.0, 13.4, 180.0);
        printf("[cov] ok grove %d", id);
    }
    else if (!strcmp(tok[0], "enex"))
    {
        CovPlace(id, 1833.4, -1681.75, 13.4, 270.0);
        printf("[cov] ok enex %d", id);
    }
    else if (!strcmp(tok[0], "car"))
    {
        if (gCar[id] != INVALID_VEHICLE_ID) DestroyVehicle(gCar[id]);
        CovPlace(id, 2490.0, -1670.0, 13.4, 270.0);
        gCar[id] = CreateVehicle(560, 2492.5, -1670.0, 13.4, 0.0, 1, 1, -1);
        SetVehicleVirtualWorld(gCar[id], GetPlayerVirtualWorld(id));
        printf("[cov] ok car %d veh=%d", id, gCar[id]);
    }
    else if (!strcmp(tok[0], "info"))
    {
        new Float:x, Float:y, Float:z;
        GetPlayerPos(id, x, y, z);
        printf("[cov] info id=%d st=%d int=%d vw=%d veh=%d pos=%.2f,%.2f,%.2f", id, GetPlayerState(id),
            GetPlayerInterior(id), GetPlayerVirtualWorld(id), GetPlayerVehicleID(id), x, y, z);
    }
    else printf("[cov] error bad command %s", tok[0]);
    return 1;
}
