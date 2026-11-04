// TRACE-4 filterscript (trace lane only): SP-OFF baseline.
// When the traced player spawns, put it in the middle of Grove Street (busy single-player
// area: gang peds, traffic, parked cars) and leave it there for IDLE_MS. Then move it to the
// outside spawn point of the "BAR1" entry-exit (Alhambra club, data/maps/LA/LAe.ipl, a
// marker 2.5 m east of that point), facing the marker, so the scenario can walk it in with
// W: the interior transition is where GTA generates ambient interior peds.
// Logs "[trace-fs] ..." lines. Compile with IDLE_MS=<ms> on the pawncc command line.
#define FILTERSCRIPT
#include <open.mp>

#if !defined IDLE_MS
    #define IDLE_MS 300000
#endif

public OnFilterScriptInit() { printf("[trace-fs] spoff loaded, idle %d ms", IDLE_MS); return 1; }

public OnPlayerSpawn(playerid)
{
    if (IsPlayerNPC(playerid)) return 1;
    SetTimerEx("SpoffGrove", 500, false, "i", playerid);
    SetTimerEx("SpoffEnex", 500 + IDLE_MS, false, "i", playerid);
    return 1;
}

forward SpoffGrove(playerid);
public SpoffGrove(playerid)
{
    if (!IsPlayerConnected(playerid)) return 1;
    // The gungame gamemode limits the world to its arena on spawn; lift that.
    SetPlayerWorldBounds(playerid, 20000.0, -20000.0, 20000.0, -20000.0);
    SetPlayerInterior(playerid, 0);
    SetPlayerPos(playerid, 2495.0, -1675.0, 13.4);
    SetPlayerFacingAngle(playerid, 180.0);
    SetCameraBehindPlayer(playerid);
    printf("[trace-fs] grove %d", playerid);
    return 1;
}

forward SpoffEnex(playerid);
public SpoffEnex(playerid)
{
    if (!IsPlayerConnected(playerid)) return 1;
    SetPlayerInterior(playerid, 0);
    SetPlayerPos(playerid, 1833.4, -1681.75, 13.4);
    SetPlayerFacingAngle(playerid, 270.0); // facing +x (east), towards the marker
    SetCameraBehindPlayer(playerid);
    printf("[trace-fs] enex %d", playerid);
    return 1;
}
