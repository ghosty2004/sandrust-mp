// DIFF-3 cross-client filterscript (diff lane only).
//
// Driven by tools/diff/xclient.py over RCON (OnRconCommand). Every command ends
// with " ;": RCON text arriving through the query port has stale bytes after its
// real length, so only the tokens before the ';' are used.
//   xc tp <id> <x> <y> <z> <angle> ;   SetPlayerPos + SetPlayerFacingAngle + SetCameraBehindPlayer
//   xc hpar <id> <health> <armour> ;   SetPlayerHealth + SetPlayerArmour
//   xc skin <id> <skin> ;              SetPlayerSkin
//   xc sa <id> <action> ;              SetPlayerSpecialAction
//   xc weapon <id> <weapon> <ammo> ;   GivePlayerWeapon (M7)
//   xc car <id> ;                      (re)creates that player's Sultan 2.5 m north of it (heading 90, driver door
//                                      towards the player) and turns the player to face it (angle 0)
//   xc putin <id> <owner> <seat> ;     PutPlayerInVehicle(id, car of <owner>, seat)
//   xc comp <owner> <component> ;      AddVehicleComponent(car of <owner>, component)
//   xc out <id> ;                      RemovePlayerFromVehicle
//   xc truck <id> <x> <y> <z> ;        (re)creates that player's Linerunner (403) at x,y,z heading 90 and an
//                                      Article Trailer (435) 10 m behind it (east), not attached
//   xc tputin <id> ;                   PutPlayerInVehicle(id, its truck, 0)
//   xc hitch <id> ;                    AttachTrailerToVehicle(its trailer, its truck)
//   xc gobj <model> <x> <y> <z> <rz> ; CreateObject (global; destroyed with the filterscript)
//   xc vobj <owner> <model> ;          CreateObject + AttachObjectToVehicle(car of <owner>, 0, -0.5, 0.9, 0, 0, 30)
//   xc pobj <id> <model> ;             SetPlayerAttachedObject(id, 0, model, bone 2 (head), 0.1, 0.02, 0, 0, 0, 0)
//   xc bubble <id> <tag> ;             SetPlayerChatBubble(id, "bubble of <tag>", 0xFFAA00FF, 50.0, 15000)
//   xc info <tag> ;                    one line per connected player (the server's view):
//     [xc] info <tag> id=.. name=.. st=.. pos=x,y,z ang=.. skin=.. hp=.. ar=.. anim=.. sa=..
//   and one per vehicle the filterscript made:
//     [xc] vinfo <tag> owner=.. kind=car|truck|trailer id=.. model=.. pos=x,y,z ang=.. trailer=.. streamed=<bit per player>
// Shots and damage are logged for the combat-hit steps:
//   [xc] shot id=.. weapon=.. hittype=.. hitid=.. at=x,y,z  (OnPlayerWeaponShot)
//   [xc] takedamage id=.. issuer=.. amount=.. weapon=.. part=..   (OnPlayerTakeDamage)
//   [xc] givedamage id=.. damaged=.. amount=.. weapon=.. part=..  (OnPlayerGiveDamage)
//   [xc] unocc veh=.. id=.. seat=.. pos=x,y,z                  (OnUnoccupiedVehicleUpdate: who sent 209)
// Each command prints "[xc] ok <cmd> ..." or "[xc] error ..." when done.
#define FILTERSCRIPT
#include <open.mp>

new gXcCar[MAX_PLAYERS] = { INVALID_VEHICLE_ID, ... };
new gXcTruck[MAX_PLAYERS] = { INVALID_VEHICLE_ID, ... };
new gXcTrailer[MAX_PLAYERS] = { INVALID_VEHICLE_ID, ... };
new gXcObjs[16] = { INVALID_OBJECT_ID, ... };
new gXcObjCount = 0;

stock XcKeepObject(obj)
{
    if (gXcObjCount < sizeof gXcObjs) { gXcObjs[gXcObjCount] = obj; gXcObjCount++; }
}

stock XcVinfo(const tag[], owner, const kind[], veh)
{
    if (veh == INVALID_VEHICLE_ID) return;
    new Float:x, Float:y, Float:z, Float:a;
    GetVehiclePos(veh, x, y, z); GetVehicleZAngle(veh, a);
    // streamed: bit i set = streamed in for player i (players 0..30)
    new streamed = 0;
    for (new i = 0; i < 31; i++) if (IsPlayerConnected(i) && IsVehicleStreamedIn(veh, i)) streamed |= 1 << i;
    printf("[xc] vinfo %s owner=%d kind=%s id=%d model=%d pos=%.3f,%.3f,%.3f ang=%.2f trailer=%d streamed=%d",
        tag, owner, kind, veh, GetVehicleModel(veh), x, y, z, a, GetVehicleTrailer(veh), streamed);
}

public OnPlayerWeaponShot(playerid, WEAPON:weaponid, BULLET_HIT_TYPE:hittype, hitid, Float:fX, Float:fY, Float:fZ)
{
    printf("[xc] shot id=%d weapon=%d hittype=%d hitid=%d at=%.3f,%.3f,%.3f", playerid, _:weaponid, _:hittype, hitid, fX, fY, fZ);
    return 1;
}

public OnUnoccupiedVehicleUpdate(vehicleid, playerid, passenger_seat, Float:new_x, Float:new_y, Float:new_z, Float:vel_x, Float:vel_y, Float:vel_z)
{
    printf("[xc] unocc veh=%d id=%d seat=%d pos=%.3f,%.3f,%.3f", vehicleid, playerid, passenger_seat, new_x, new_y, new_z);
    return 1;
}

public OnPlayerTakeDamage(playerid, issuerid, Float:amount, WEAPON:weaponid, bodypart)
{
    printf("[xc] takedamage id=%d issuer=%d amount=%.2f weapon=%d part=%d", playerid, issuerid, amount, _:weaponid, bodypart);
    return 1;
}

public OnPlayerGiveDamage(playerid, damagedid, Float:amount, WEAPON:weaponid, bodypart)
{
    printf("[xc] givedamage id=%d damaged=%d amount=%.2f weapon=%d part=%d", playerid, damagedid, amount, _:weaponid, bodypart);
    return 1;
}

public OnFilterScriptInit() { print("[xc] xclient loaded"); return 1; }

public OnFilterScriptExit()
{
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (gXcCar[i] != INVALID_VEHICLE_ID) DestroyVehicle(gXcCar[i]);
        if (gXcTruck[i] != INVALID_VEHICLE_ID) DestroyVehicle(gXcTruck[i]);
        if (gXcTrailer[i] != INVALID_VEHICLE_ID) DestroyVehicle(gXcTrailer[i]);
    }
    for (new i = 0; i < gXcObjCount; i++) DestroyObject(gXcObjs[i]);
    return 1;
}

// Splits cmd (up to ';' or NUL) on spaces into tok[][]; returns the count.
stock XcTokens(const cmd[], tok[][], maxtok, toklen)
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

stock XcInfo(const tag[])
{
    new name[MAX_PLAYER_NAME + 1], Float:x, Float:y, Float:z, Float:a, Float:hp, Float:ar;
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (!IsPlayerConnected(i)) continue;
        GetPlayerName(i, name, sizeof name);
        GetPlayerPos(i, x, y, z); GetPlayerFacingAngle(i, a);
        GetPlayerHealth(i, hp); GetPlayerArmour(i, ar);
        printf("[xc] info %s id=%d name=%s st=%d pos=%.3f,%.3f,%.3f ang=%.2f skin=%d hp=%.1f ar=%.1f anim=%d sa=%d veh=%d seat=%d",
            tag, i, name, GetPlayerState(i), x, y, z, a, GetPlayerSkin(i), hp, ar,
            GetPlayerAnimationIndex(i), _:GetPlayerSpecialAction(i), GetPlayerVehicleID(i), GetPlayerVehicleSeat(i));
    }
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        XcVinfo(tag, i, "car", gXcCar[i]);
        XcVinfo(tag, i, "truck", gXcTruck[i]);
        XcVinfo(tag, i, "trailer", gXcTrailer[i]);
    }
    printf("[xc] ok info %s", tag);
}

public OnRconCommand(cmd[])
{
    if (strcmp(cmd, "xc ", false, 3)) return 0;
    new tok[8][24];
    new n = XcTokens(cmd[3], tok, sizeof tok, sizeof tok[]);
    if (n < 1) { print("[xc] error empty command"); return 1; }
    if (!strcmp(tok[0], "info"))
    {
        XcInfo(n >= 2 ? tok[1] : "-");
        return 1;
    }
    if (!strcmp(tok[0], "gobj") && n >= 6)
    {
        new obj = CreateObject(strval(tok[1]), floatstr(tok[2]), floatstr(tok[3]), floatstr(tok[4]), 0.0, 0.0, floatstr(tok[5]));
        XcKeepObject(obj);
        printf("[xc] ok gobj %d", obj);
        return 1;
    }
    if (n < 2) { printf("[xc] error %s needs a player id", tok[0]); return 1; }
    new id = strval(tok[1]);
    if (!IsPlayerConnected(id)) { printf("[xc] error %s: player %d not connected", tok[0], id); return 1; }
    if (!strcmp(tok[0], "tp") && n >= 6)
    {
        SetPlayerPos(id, floatstr(tok[2]), floatstr(tok[3]), floatstr(tok[4]));
        SetPlayerFacingAngle(id, floatstr(tok[5]));
        SetCameraBehindPlayer(id);
        printf("[xc] ok tp %d", id);
    }
    else if (!strcmp(tok[0], "car"))
    {
        new Float:x, Float:y, Float:z;
        GetPlayerPos(id, x, y, z);
        if (gXcCar[id] != INVALID_VEHICLE_ID) DestroyVehicle(gXcCar[id]);
        gXcCar[id] = CreateVehicle(560, x, y + 2.5, z, 90.0, 1, 1, -1);
        SetPlayerFacingAngle(id, 0.0);
        SetCameraBehindPlayer(id);
        printf("[xc] ok car %d veh=%d", id, gXcCar[id]);
    }
    else if (!strcmp(tok[0], "putin") && n >= 4)
    {
        new owner = strval(tok[2]);
        if (owner < 0 || owner >= MAX_PLAYERS || gXcCar[owner] == INVALID_VEHICLE_ID)
        {
            printf("[xc] error putin: player %d has no car", owner);
            return 1;
        }
        PutPlayerInVehicle(id, gXcCar[owner], strval(tok[3]));
        printf("[xc] ok putin %d", id);
    }
    else if (!strcmp(tok[0], "comp") && n >= 3)
    {
        if (gXcCar[id] == INVALID_VEHICLE_ID) { printf("[xc] error comp: player %d has no car", id); return 1; }
        AddVehicleComponent(gXcCar[id], strval(tok[2]));
        printf("[xc] ok comp %d", id);
    }
    else if (!strcmp(tok[0], "out"))
    {
        RemovePlayerFromVehicle(id);
        printf("[xc] ok out %d", id);
    }
    else if (!strcmp(tok[0], "truck") && n >= 5)
    {
        new Float:x = floatstr(tok[2]), Float:y = floatstr(tok[3]), Float:z = floatstr(tok[4]);
        if (gXcTruck[id] != INVALID_VEHICLE_ID) DestroyVehicle(gXcTruck[id]);
        if (gXcTrailer[id] != INVALID_VEHICLE_ID) DestroyVehicle(gXcTrailer[id]);
        gXcTruck[id] = CreateVehicle(403, x, y, z, 90.0, 1, 1, -1);
        gXcTrailer[id] = CreateVehicle(435, x + 10.0, y, z, 90.0, 1, 1, -1);
        printf("[xc] ok truck %d veh=%d trailer=%d", id, gXcTruck[id], gXcTrailer[id]);
    }
    else if (!strcmp(tok[0], "tputin"))
    {
        if (gXcTruck[id] == INVALID_VEHICLE_ID) { printf("[xc] error tputin: player %d has no truck", id); return 1; }
        PutPlayerInVehicle(id, gXcTruck[id], 0);
        printf("[xc] ok tputin %d", id);
    }
    else if (!strcmp(tok[0], "hitch"))
    {
        if (gXcTruck[id] == INVALID_VEHICLE_ID) { printf("[xc] error hitch: player %d has no truck", id); return 1; }
        AttachTrailerToVehicle(gXcTrailer[id], gXcTruck[id]);
        printf("[xc] ok hitch %d", id);
    }
    else if (!strcmp(tok[0], "vobj") && n >= 3)
    {
        if (gXcCar[id] == INVALID_VEHICLE_ID) { printf("[xc] error vobj: player %d has no car", id); return 1; }
        new obj = CreateObject(strval(tok[2]), 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        XcKeepObject(obj);
        AttachObjectToVehicle(obj, gXcCar[id], 0.0, -0.5, 0.9, 0.0, 0.0, 30.0);
        printf("[xc] ok vobj %d obj=%d", id, obj);
    }
    else if (!strcmp(tok[0], "pobj") && n >= 3)
    {
        SetPlayerAttachedObject(id, 0, strval(tok[2]), 2, 0.1, 0.02, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        printf("[xc] ok pobj %d", id);
    }
    else if (!strcmp(tok[0], "bubble") && n >= 3)
    {
        new text[32];
        format(text, sizeof text, "bubble of %s", tok[2]);
        SetPlayerChatBubble(id, text, 0xFFAA00FF, 50.0, 15000);
        printf("[xc] ok bubble %d", id);
    }
    else if (!strcmp(tok[0], "weapon") && n >= 4)
    {
        GivePlayerWeapon(id, WEAPON:strval(tok[2]), strval(tok[3]));
        printf("[xc] ok weapon %d", id);
    }
    else if (!strcmp(tok[0], "hpar") && n >= 4)
    {
        SetPlayerHealth(id, floatstr(tok[2]));
        SetPlayerArmour(id, floatstr(tok[3]));
        printf("[xc] ok hpar %d", id);
    }
    else if (!strcmp(tok[0], "skin") && n >= 3)
    {
        SetPlayerSkin(id, strval(tok[2]));
        printf("[xc] ok skin %d", id);
    }
    else if (!strcmp(tok[0], "spawn"))
    {
        SpawnPlayer(id);
        printf("[xc] ok spawn %d", id);
    }
    else if (!strcmp(tok[0], "sa") && n >= 3)
    {
        SetPlayerSpecialAction(id, SPECIAL_ACTION:strval(tok[2]));
        printf("[xc] ok sa %d", id);
    }
    else printf("[xc] error bad command %s (%d tokens)", tok[0], n);
    return 1;
}
