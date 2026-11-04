// e2e probe filterscript: logs player lifecycle and state once per second
// so end-to-end tests can assert server-side observations from the log.
#define FILTERSCRIPT
#include <open.mp>

public OnFilterScriptInit() { SetTimer("E2EProbe", 1000, true); print("[e2e] probe loaded"); return 1; }

public OnPlayerConnect(playerid)
{
    new name[MAX_PLAYER_NAME + 1]; GetPlayerName(playerid, name, sizeof name);
    printf("[e2e] connect id=%d name=%s npc=%d", playerid, name, IsPlayerNPC(playerid));
    return 1;
}
public OnPlayerDisconnect(playerid, reason) { printf("[e2e] disconnect id=%d reason=%d", playerid, reason); return 1; }
public OnPlayerRequestClass(playerid, classid) { printf("[e2e] class id=%d class=%d", playerid, classid); return 1; }
public OnPlayerSpawn(playerid) { printf("[e2e] spawn id=%d", playerid); return 1; }
public OnPlayerStreamIn(playerid, forplayerid) { printf("[e2e] streamin id=%d for=%d", playerid, forplayerid); return 1; }
public OnPlayerStreamOut(playerid, forplayerid) { printf("[e2e] streamout id=%d for=%d", playerid, forplayerid); return 1; }

forward E2EProbe();
public E2EProbe()
{
    new Float:x, Float:y, Float:z, Float:a, Float:hp;
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (!IsPlayerConnected(i)) continue;
        GetPlayerPos(i, x, y, z); GetPlayerFacingAngle(i, a); GetPlayerHealth(i, hp);
        printf("[e2e] state id=%d st=%d pos=%.2f,%.2f,%.2f ang=%.1f hp=%.0f", i, GetPlayerState(i), x, y, z, a, hp);
    }
}
