// M5 e2e helper filterscript (vehicles). Loaded next to e2eprobe by tools/e2e/m5.sh.
// * The first spawned non-bot player is put on the Verdant Meadows airstrip with a Sultan (car A) next to it,
//   facing west along the runway, driver door towards the player. (TRACE-4 used the LS airport tarmac with the
//   single-player systems off; without them airport traffic and landing planes hit the test car there.)
// * "DriveBot" (samp-bot example drive_bot) spawns → car B is created 50 m north-east of the player (in view of the camera) and the
//   bot is put into it as driver; drive_bot then sends driver sync on a 10 m circle.
// * Every 500 ms: "[m5] veh id=… model=… pos=x,y,z driver=…" for both cars (server-side position).
// Logs "[m5] ..." lines.
#define FILTERSCRIPT
#include <open.mp>

#define LOCAL_X 300.0
#define LOCAL_Y 2500.0
#define GROUND_Z 17.0

new gLocal = INVALID_PLAYER_ID, gBot = INVALID_PLAYER_ID;
new gCarA = INVALID_VEHICLE_ID, gCarB = INVALID_VEHICLE_ID;

static bool:IsDriveBot(playerid)
{
    new name[MAX_PLAYER_NAME + 1];
    GetPlayerName(playerid, name, sizeof name);
    return strfind(name, "DriveBot") == 0;
}

public OnFilterScriptInit() { SetTimer("M5Report", 500, true); print("[m5] helper loaded"); return 1; }

public OnPlayerSpawn(playerid)
{
    if (IsDriveBot(playerid))
    {
        gBot = playerid;
        SetTimer("M5BotCar", 500, false);
        SetTimer("M5BotIn", 1500, false);
    }
    else if (gLocal == INVALID_PLAYER_ID)
    {
        gLocal = playerid;
        SetTimer("M5LocalSetup", 1500, false);
    }
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    if (playerid == gBot) gBot = INVALID_PLAYER_ID;
    return 1;
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[m5] state %d %d->%d veh %d", playerid, _:oldstate, _:newstate, GetPlayerVehicleID(playerid));
    return 1;
}

public OnPlayerEnterVehicle(playerid, vehicleid, ispassenger)
{
    printf("[m5] enter-request %d veh %d passenger %d", playerid, vehicleid, ispassenger);
    return 1;
}

public OnPlayerExitVehicle(playerid, vehicleid)
{
    printf("[m5] exit-request %d veh %d", playerid, vehicleid);
    return 1;
}

forward M5LocalSetup();
public M5LocalSetup()
{
    if (!IsPlayerConnected(gLocal)) return 1;
    // The gungame gamemode limits the world to its arena on spawn; lift that.
    SetPlayerWorldBounds(gLocal, 20000.0, -20000.0, 20000.0, -20000.0);
    SetPlayerInterior(gLocal, 0);
    SetPlayerVirtualWorld(gLocal, 0);
    SetPlayerPos(gLocal, LOCAL_X, LOCAL_Y - 2.5, GROUND_Z);
    SetPlayerFacingAngle(gLocal, 0.0); // facing +y, towards car A's driver door
    SetCameraBehindPlayer(gLocal);
    if (gCarA == INVALID_VEHICLE_ID)
    {
        gCarA = CreateVehicle(560, LOCAL_X, LOCAL_Y, GROUND_Z, 90.0, 1, 1, -1);
        // M6-P: a plate text that reaches the client with the stream-in (RPC 123 after 164).
        SetVehicleNumberPlate(gCarA, "M5 RUST");
    }
    printf("[m5] local ready id=%d carA=%d", gLocal, gCarA);
    return 1;
}

forward M5BotCar();
public M5BotCar()
{
    if (!IsPlayerConnected(gBot)) return 1;
    SetPlayerWorldBounds(gBot, 20000.0, -20000.0, 20000.0, -20000.0);
    SetPlayerInterior(gBot, 0);
    SetPlayerVirtualWorld(gBot, 0);
    SetPlayerPos(gBot, LOCAL_X + 30.0, LOCAL_Y + 40.0, GROUND_Z);
    if (gCarB == INVALID_VEHICLE_ID) gCarB = CreateVehicle(560, LOCAL_X + 30.0, LOCAL_Y + 40.0, GROUND_Z, 0.0, 3, 3, -1);
    printf("[m5] bot car id=%d carB=%d", gBot, gCarB);
    return 1;
}

forward M5BotIn();
public M5BotIn()
{
    if (!IsPlayerConnected(gBot) || gCarB == INVALID_VEHICLE_ID) return 1;
    PutPlayerInVehicle(gBot, gCarB, 0);
    printf("[m5] bot in id=%d carB=%d", gBot, gCarB);
    return 1;
}

forward M5Report();
public M5Report()
{
    new cars[2]; cars[0] = gCarA; cars[1] = gCarB;
    for (new c = 0; c < 2; c++)
    {
        if (cars[c] == INVALID_VEHICLE_ID) continue;
        new Float:x, Float:y, Float:z, driver = INVALID_PLAYER_ID;
        GetVehiclePos(cars[c], x, y, z);
        for (new i = 0; i < MAX_PLAYERS; i++)
        {
            if (IsPlayerConnected(i) && GetPlayerVehicleID(i) == cars[c] && GetPlayerVehicleSeat(i) == 0) driver = i;
        }
        printf("[m5] veh id=%d model=%d pos=%.2f,%.2f,%.2f driver=%d", cars[c], GetVehicleModel(cars[c]), x, y, z, driver);
    }
    return 1;
}
