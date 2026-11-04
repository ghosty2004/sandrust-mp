// BOT-2 e2e helper filterscript (loaded next to e2eprobe by tools/e2e/bot2.sh).
// Test bots are recognised by name prefix; 1.5 s after spawning (after the
// gamemode's own spawn placement) they are set up:
//   B2Drive* / B2Pass*  placed at a fixed spot, a Sultan (560) is created 4 m to
//                       their right ("[b2] vehicle id=… for=… pos=…").
//   B2Target*           placed at TARGET; health is reset to 100 every 500 ms.
//   B2Shoot*            placed 8 m south of TARGET facing it, given a Deagle.
// Logs (one line each, prefix "[b2]"): statechange, enter, exit, shot, takedamage,
// givedamage, death, and every 500 ms the position of each created vehicle.
#define FILTERSCRIPT
#include <open.mp>

#define TARGET_X (-1290.0)
#define TARGET_Y (2520.0)
#define TARGET_Z (87.0)

new gVeh[MAX_PLAYERS];
new gTarget = INVALID_PLAYER_ID;

forward B2Setup(playerid);
forward B2Tick();

static bool:HasPrefix(playerid, const prefix[])
{
    new name[MAX_PLAYER_NAME + 1];
    GetPlayerName(playerid, name, sizeof name);
    return strfind(name, prefix) == 0;
}

public OnFilterScriptInit()
{
    for (new i = 0; i < MAX_PLAYERS; i++) gVeh[i] = INVALID_VEHICLE_ID;
    SetTimer("B2Tick", 500, true);
    print("[b2] helper loaded");
    return 1;
}

public OnPlayerSpawn(playerid)
{
    if (HasPrefix(playerid, "B2")) SetTimerEx("B2Setup", 1500, false, "i", playerid);
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    if (gVeh[playerid] != INVALID_VEHICLE_ID)
    {
        DestroyVehicle(gVeh[playerid]);
        gVeh[playerid] = INVALID_VEHICLE_ID;
    }
    if (gTarget == playerid) gTarget = INVALID_PLAYER_ID;
    return 1;
}

public B2Setup(playerid)
{
    if (!IsPlayerConnected(playerid)) return 0;
    if (HasPrefix(playerid, "B2Drive") || HasPrefix(playerid, "B2Pass"))
    {
        new Float:x = HasPrefix(playerid, "B2Drive") ? -1300.0 : -1330.0, Float:y = 2500.0, Float:z = 87.0;
        SetPlayerPos(playerid, x, y, z);
        SetPlayerFacingAngle(playerid, 0.0);
        if (gVeh[playerid] != INVALID_VEHICLE_ID) DestroyVehicle(gVeh[playerid]);
        gVeh[playerid] = CreateVehicle(560, x + 4.0, y, z, 0.0, 1, 1, -1);
        printf("[b2] vehicle id=%d for=%d model=560 pos=%.2f,%.2f,%.2f", gVeh[playerid], playerid, x + 4.0, y, z);
    }
    else if (HasPrefix(playerid, "B2Target"))
    {
        SetPlayerPos(playerid, TARGET_X, TARGET_Y, TARGET_Z);
        SetPlayerFacingAngle(playerid, 180.0);
        gTarget = playerid;
        printf("[b2] target id=%d", playerid);
    }
    else if (HasPrefix(playerid, "B2Shoot"))
    {
        SetPlayerPos(playerid, TARGET_X, TARGET_Y - 8.0, TARGET_Z);
        SetPlayerFacingAngle(playerid, 0.0);
        GivePlayerWeapon(playerid, WEAPON_DEAGLE, 100);
        printf("[b2] shooter id=%d", playerid);
    }
    return 1;
}

public B2Tick()
{
    new Float:x, Float:y, Float:z;
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (gVeh[i] == INVALID_VEHICLE_ID) continue;
        GetVehiclePos(gVeh[i], x, y, z);
        printf("[b2] veh id=%d pos=%.2f,%.2f,%.2f", gVeh[i], x, y, z);
    }
    if (gTarget != INVALID_PLAYER_ID && IsPlayerConnected(gTarget)) SetPlayerHealth(gTarget, 100.0);
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[b2] statechange id=%d new=%d old=%d", playerid, _:newstate, _:oldstate);
    return 1;
}

public OnPlayerEnterVehicle(playerid, vehicleid, ispassenger)
{
    printf("[b2] enter id=%d veh=%d passenger=%d", playerid, vehicleid, ispassenger);
    return 1;
}

public OnPlayerExitVehicle(playerid, vehicleid)
{
    printf("[b2] exit id=%d veh=%d", playerid, vehicleid);
    return 1;
}

public OnPlayerWeaponShot(playerid, WEAPON:weaponid, BULLET_HIT_TYPE:hittype, hitid, Float:fX, Float:fY, Float:fZ)
{
    printf("[b2] shot id=%d weapon=%d hittype=%d hitid=%d off=%.3f,%.3f,%.3f", playerid, _:weaponid, _:hittype, hitid, fX, fY, fZ);
    return 1;
}

public OnPlayerTakeDamage(playerid, issuerid, Float:amount, WEAPON:weaponid, bodypart)
{
    printf("[b2] takedamage id=%d issuer=%d amount=%.2f weapon=%d bodypart=%d", playerid, issuerid, amount, _:weaponid, bodypart);
    return 1;
}

public OnPlayerGiveDamage(playerid, damagedid, Float:amount, WEAPON:weaponid, bodypart)
{
    printf("[b2] givedamage id=%d damaged=%d amount=%.2f weapon=%d bodypart=%d", playerid, damagedid, amount, _:weaponid, bodypart);
    return 1;
}

public OnPlayerDeath(playerid, killerid, WEAPON:reason)
{
    printf("[b2] death id=%d killer=%d reason=%d", playerid, killerid, _:reason);
    return 1;
}
