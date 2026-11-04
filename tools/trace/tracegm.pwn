// TRACE-6: neutral gamemode for combat measurements (trace lane only). One class at the LS airport
// tarmac, no weapons, no death handling, so the client's own death/respawn flow is visible
// (the bundled gungame spectates the killer after a death). Selected with TRACE_GAMEMODE=tracegm.
#include <open.mp>

main() {}

public OnGameModeInit()
{
    SetGameModeText("trace");
    AddPlayerClass(0, 1700.0, -2600.0, 13.55, 0.0, WEAPON_FIST, 0, WEAPON_FIST, 0, WEAPON_FIST, 0);
    return 1;
}

public OnPlayerRequestClass(playerid, classid)
{
    SetPlayerPos(playerid, 1700.0, -2600.0, 13.55);
    SetPlayerCameraPos(playerid, 1700.0, -2610.0, 16.0);
    SetPlayerCameraLookAt(playerid, 1700.0, -2600.0, 13.55);
    return 1;
}
