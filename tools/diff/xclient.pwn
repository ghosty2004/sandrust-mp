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
//   xc info <tag> ;                    one line per connected player (the server's view):
//     [xc] info <tag> id=.. name=.. st=.. pos=x,y,z ang=.. skin=.. hp=.. ar=.. anim=.. sa=..
// Each command prints "[xc] ok <cmd> ..." or "[xc] error ..." when done.
#define FILTERSCRIPT
#include <open.mp>

new gXcCar[MAX_PLAYERS] = { INVALID_VEHICLE_ID, ... };

public OnFilterScriptInit() { print("[xc] xclient loaded"); return 1; }

public OnFilterScriptExit()
{
    for (new i = 0; i < MAX_PLAYERS; i++) if (gXcCar[i] != INVALID_VEHICLE_ID) DestroyVehicle(gXcCar[i]);
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
