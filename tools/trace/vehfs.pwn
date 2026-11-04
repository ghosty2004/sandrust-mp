// TRACE-4 filterscript (trace lane only): vehicles.
// * Traced player spawns → it is put on the LS airport tarmac with a Sultan (car A) next to
//   it, driver door towards the player; the scenario enters (F), drives (W) and exits (F).
// * "DriveBot" (samp-bot example drive_bot) spawns → car B is created 40 m away and the bot is
//   put into it as driver (PutPlayerInVehicle); it then sends driver sync on a 10 m circle.
//   Timeline after the bot's spawn: +21.5 s RemovePlayerFromVehicle(bot), +26.5 s put back in,
//   +31.5 s the traced player is moved 1500 m away (stream-out), +41.5 s brought back
//   (stream-in again), +51.5 s DestroyVehicle(B) with the bot in it.
// Logs "[trace-fs] ..." lines.
#define FILTERSCRIPT
#include <open.mp>

#define LOCAL_X 1700.0
#define LOCAL_Y -2600.0
#define GROUND_Z 13.55

new gLocal = INVALID_PLAYER_ID, gBot = INVALID_PLAYER_ID;
new gCarA = INVALID_VEHICLE_ID, gCarB = INVALID_VEHICLE_ID;

public OnFilterScriptInit() { print("[trace-fs] vehfs loaded"); return 1; }

public OnPlayerSpawn(playerid)
{
    new name[MAX_PLAYER_NAME + 1]; GetPlayerName(playerid, name, sizeof name);
    if (!strcmp(name, "DriveBot"))
    {
        gBot = playerid;
        SetTimer("VehBotCar", 500, false);
        SetTimer("VehBotIn", 1500, false);
        SetTimer("VehBotOut", 21500, false);
        SetTimer("VehBotIn", 26500, false);
        SetTimer("VehLocalAway", 31500, false);
        SetTimer("VehLocalBack", 41500, false);
        SetTimer("VehDestroyB", 51500, false);
    }
    else if (gLocal == INVALID_PLAYER_ID)
    {
        gLocal = playerid;
        SetTimer("VehLocalSetup", 500, false);
    }
    return 1;
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[trace-fs] state %d %d->%d veh %d", playerid, _:oldstate, _:newstate, GetPlayerVehicleID(playerid));
    return 1;
}

public OnPlayerEnterVehicle(playerid, vehicleid, ispassenger)
{
    printf("[trace-fs] enter-request %d veh %d passenger %d", playerid, vehicleid, ispassenger);
    return 1;
}

public OnPlayerExitVehicle(playerid, vehicleid)
{
    printf("[trace-fs] exit-request %d veh %d", playerid, vehicleid);
    return 1;
}

forward VehLocalSetup();
public VehLocalSetup()
{
    if (!IsPlayerConnected(gLocal)) return 1;
    // The gungame gamemode limits the world to its arena on spawn; lift that.
    SetPlayerWorldBounds(gLocal, 20000.0, -20000.0, 20000.0, -20000.0);
    SetPlayerInterior(gLocal, 0);
    SetPlayerPos(gLocal, LOCAL_X, LOCAL_Y, GROUND_Z);
    SetPlayerFacingAngle(gLocal, 270.0); // facing +x, towards car A's driver door
    SetCameraBehindPlayer(gLocal);
    gCarA = CreateVehicle(560, LOCAL_X + 2.5, LOCAL_Y, GROUND_Z, 0.0, 1, 1, -1);
    printf("[trace-fs] veh ready local %d carA %d", gLocal, gCarA);
    return 1;
}

forward VehBotCar();
public VehBotCar()
{
    if (!IsPlayerConnected(gBot)) return 1;
    SetPlayerWorldBounds(gBot, 20000.0, -20000.0, 20000.0, -20000.0);
    SetPlayerInterior(gBot, 0);
    SetPlayerPos(gBot, LOCAL_X + 20.0, LOCAL_Y + 40.0, GROUND_Z);
    gCarB = CreateVehicle(560, LOCAL_X + 20.0, LOCAL_Y + 40.0, GROUND_Z, 0.0, 3, 3, -1);
    printf("[trace-fs] bot car %d", gCarB);
    return 1;
}

forward VehBotIn();
public VehBotIn()
{
    if (!IsPlayerConnected(gBot) || gCarB == INVALID_VEHICLE_ID) return 1;
    PutPlayerInVehicle(gBot, gCarB, 0);
    printf("[trace-fs] bot in %d", gCarB);
    return 1;
}

forward VehBotOut();
public VehBotOut()
{
    if (!IsPlayerConnected(gBot)) return 1;
    RemovePlayerFromVehicle(gBot);
    printf("[trace-fs] bot out");
    return 1;
}

forward VehLocalAway();
public VehLocalAway()
{
    if (!IsPlayerConnected(gLocal)) return 1;
    SetPlayerPos(gLocal, LOCAL_X + 1500.0, LOCAL_Y, GROUND_Z + 200.0);
    TogglePlayerControllable(gLocal, false);
    printf("[trace-fs] local away");
    return 1;
}

forward VehLocalBack();
public VehLocalBack()
{
    if (!IsPlayerConnected(gLocal)) return 1;
    SetPlayerPos(gLocal, LOCAL_X, LOCAL_Y, GROUND_Z);
    TogglePlayerControllable(gLocal, true);
    printf("[trace-fs] local back");
    return 1;
}

forward VehDestroyB();
public VehDestroyB()
{
    if (gCarB == INVALID_VEHICLE_ID) return 1;
    DestroyVehicle(gCarB);
    printf("[trace-fs] carB %d destroyed", gCarB);
    gCarB = INVALID_VEHICLE_ID;
    return 1;
}
