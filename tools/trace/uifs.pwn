// TRACE-3/TRACE-5 filterscript (trace lane only): server-side actions on demand.
// tools/trace/ui.sh appends lines "SEQ CMD|ARG|ARG|..." to scriptfiles/uicmd.txt; every
// 100 ms this script runs the lines with a SEQ above the last one it ran and prints
// "[ui-fs] SEQ done ..." (the scenario waits for that line). In texts, "\n" and "\t" are
// turned into newline / tab. Players: "L" = the traced (first non-bot) player, "B1".."B4" =
// bots in join order, a number = that id, "-" = INVALID_PLAYER_ID.
//   msg|RRGGBBAA|text                      SendClientMessage(L)
//   pmsg|SENDER|text                       SendPlayerMessageToPlayer(L, sender)
//   dialog|ID|STYLE|caption|body|b1|b2     ShowPlayerDialog(L)
//   gametext|STYLE|MS|text                 GameTextForPlayer(L)
//   death|KILLER|VICTIM|REASON             SendDeathMessage
//   pos|P|x|y|z|angle                      SetPlayerPos + facing (+ camera behind for L)
//   hp|P|value   armour|P|value            SetPlayerHealth / SetPlayerArmour
//   color|P|RRGGBBAA   score|P|n           SetPlayerColor / SetPlayerScore
//   bubble|P|RRGGBBAA|dist|ms|text         SetPlayerChatBubble
//   veh|model|x|y|z|angle|c1|c2[|respawn_s] CreateVehicle (prints the id)
//   putin|P|VEH|SEAT                       PutPlayerInVehicle
//   params|VEH|engine|lights|alarm|doors|bonnet|boot|objective   SetVehicleParamsEx
//   vhp|VEH|health                         SetVehicleHealth
//   vdmg|VEH|panels|doors|lights|tires     UpdateVehicleDamageStatus
//   vdel|VEH                               DestroyVehicle
//   weapon|P|weapon|ammo                   GivePlayerWeapon
//   time|h|m   weather|w   interior|P|i    SetPlayerTime / SetPlayerWeather / SetPlayerInterior
//   nametagdist|meters                     (no runtime native; logged only)
//   camera|P                               SetCameraBehindPlayer
//   togglecontrol|P|0/1                    TogglePlayerControllable
//   spectate|P|TARGET                      TogglePlayerSpectating + PlayerSpectatePlayer
//   attachtrailer|TRAILER|VEH              AttachTrailerToVehicle
//   where|P                                prints position, vehicle, state, health
//   manual                                 ManualVehicleEngineAndLights()
//   stats|P                                prints health, armour, weapon
#define FILTERSCRIPT
#pragma dynamic 65536
#include <open.mp>

new gLast = 0;
new gBots[8] = { INVALID_PLAYER_ID, ... };
new gNumBots = 0;
new gLocal = INVALID_PLAYER_ID;

public OnFilterScriptInit()
{
    new File:f = fopen("uicmd.txt", io_write);
    if (f) fclose(f);
    SetTimer("UiPoll", 100, true);
    // Trains can only be added statically (open.mp CreateVehicle refuses 537/538).
    printf("[ui-fs] static train %d", AddStaticVehicle(538, 1700.7551, -1953.6531, 14.8756, 200.0, 1, 1));
    print("[ui-fs] loaded");
    return 1;
}

public OnPlayerConnect(playerid)
{
    new name[MAX_PLAYER_NAME + 1];
    GetPlayerName(playerid, name, sizeof name);
    if (strfind(name, "Bot") != -1 && gNumBots < sizeof gBots) gBots[gNumBots++] = playerid;
    else if (gLocal == INVALID_PLAYER_ID) gLocal = playerid;
    printf("[ui-fs] connect %d %s", playerid, name);
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    if (playerid == gLocal) gLocal = INVALID_PLAYER_ID;
    printf("[ui-fs] disconnect %d %d", playerid, reason);
    return 1;
}

public OnPlayerSpawn(playerid)
{
    SetPlayerWorldBounds(playerid, 20000.0, -20000.0, 20000.0, -20000.0);
    printf("[ui-fs] spawn %d", playerid);
    return 1;
}

public OnDialogResponse(playerid, dialogid, response, listitem, inputtext[])
{
    printf("[ui-fs] dialog-response player %d id %d response %d listitem %d text '%s'", playerid, dialogid, response, listitem, inputtext);
    return 1;
}

public OnPlayerText(playerid, text[])
{
    printf("[ui-fs] text %d '%s'", playerid, text);
    return 1;
}

public OnPlayerCommandText(playerid, cmdtext[])
{
    printf("[ui-fs] command %d '%s'", playerid, cmdtext);
    return 0;
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[ui-fs] state %d %d->%d veh %d seat %d", playerid, _:oldstate, _:newstate, GetPlayerVehicleID(playerid), GetPlayerVehicleSeat(playerid));
    return 1;
}

public OnPlayerEnterVehicle(playerid, vehicleid, ispassenger)
{
    printf("[ui-fs] enter-request %d veh %d passenger %d", playerid, vehicleid, ispassenger);
    return 1;
}

public OnPlayerExitVehicle(playerid, vehicleid)
{
    printf("[ui-fs] exit-request %d veh %d", playerid, vehicleid);
    return 1;
}

public OnVehicleDamageStatusUpdate(vehicleid, playerid)
{
    new VEHICLE_PANEL_STATUS:panels, VEHICLE_DOOR_STATUS:doors, VEHICLE_LIGHT_STATUS:lights, VEHICLE_TYRE_STATUS:tyres;
    GetVehicleDamageStatus(vehicleid, panels, doors, lights, tyres);
    printf("[ui-fs] damage-status veh %d by %d panels %x doors %x lights %x tyres %x", vehicleid, playerid, _:panels, _:doors, _:lights, _:tyres);
    return 1;
}

public OnVehicleDeath(vehicleid, killerid)
{
    printf("[ui-fs] vehicle-death %d by %d", vehicleid, killerid);
    return 1;
}

public OnVehicleSpawn(vehicleid)
{
    printf("[ui-fs] vehicle-spawn %d", vehicleid);
    return 1;
}

public OnUnoccupiedVehicleUpdate(vehicleid, playerid, passenger_seat, Float:new_x, Float:new_y, Float:new_z, Float:vel_x, Float:vel_y, Float:vel_z)
{
    printf("[ui-fs] unoccupied veh %d by %d seat %d pos %.2f %.2f %.2f vel %.3f %.3f %.3f", vehicleid, playerid, passenger_seat, new_x, new_y, new_z, vel_x, vel_y, vel_z);
    return 1;
}

public OnTrailerUpdate(playerid, vehicleid)
{
    printf("[ui-fs] trailer-update %d veh %d", playerid, vehicleid);
    return 1;
}

public OnVehicleMod(playerid, vehicleid, componentid)
{
    printf("[ui-fs] mod %d veh %d comp %d", playerid, vehicleid, componentid);
    return 1;
}

public OnVehiclePaintjob(playerid, vehicleid, paintjobid)
{
    printf("[ui-fs] paintjob %d veh %d pj %d", playerid, vehicleid, paintjobid);
    return 1;
}

public OnVehicleRespray(playerid, vehicleid, color1, color2)
{
    printf("[ui-fs] respray %d veh %d %d %d", playerid, vehicleid, color1, color2);
    return 1;
}

public OnPlayerDeath(playerid, killerid, WEAPON:reason)
{
    printf("[ui-fs] death %d killer %d reason %d", playerid, killerid, _:reason);
    return 1;
}

public OnPlayerTakeDamage(playerid, issuerid, Float:amount, WEAPON:weaponid, bodypart)
{
    printf("[ui-fs] take-damage %d from %d %.3f weapon %d part %d", playerid, issuerid, amount, _:weaponid, bodypart);
    return 1;
}

public OnPlayerGiveDamage(playerid, damagedid, Float:amount, WEAPON:weaponid, bodypart)
{
    printf("[ui-fs] give-damage %d to %d %.3f weapon %d part %d", playerid, damagedid, amount, _:weaponid, bodypart);
    return 1;
}

hexval(const s[])
{
    new v = 0;
    for (new i = 0; s[i] != 0; i++)
    {
        new c = s[i], d;
        if (c >= '0' && c <= '9') d = c - '0';
        else if (c >= 'a' && c <= 'f') d = c - 'a' + 10;
        else if (c >= 'A' && c <= 'F') d = c - 'A' + 10;
        else break;
        v = (v << 4) | d;
    }
    return v;
}

unescape(s[])
{
    new j = 0;
    for (new i = 0; s[i] != 0; i++)
    {
        if (s[i] == '\\' && s[i + 1] == 'n') { s[j++] = '\n'; i++; }
        else if (s[i] == '\\' && s[i + 1] == 't') { s[j++] = '\t'; i++; }
        else s[j++] = s[i];
    }
    s[j] = 0;
}

// field n (0-based) of a '|'-separated line into dest
field(const line[], n, dest[], size = sizeof dest)
{
    new start = 0, k = 0;
    for (new i = 0; ; i++)
    {
        if (line[i] == '|' || line[i] == 0)
        {
            if (k == n)
            {
                strmid(dest, line, start, i, size);
                return 1;
            }
            if (line[i] == 0) break;
            k++;
            start = i + 1;
        }
    }
    dest[0] = 0;
    return 0;
}

player(const s[])
{
    if (s[0] == 'L') return gLocal;
    if (s[0] == '-') return INVALID_PLAYER_ID;
    if (s[0] == 'B') { new k = strval(s[1]) - 1; return (k >= 0 && k < gNumBots) ? gBots[k] : INVALID_PLAYER_ID; }
    return strval(s);
}

Float:fl(const line[], n)
{
    new b[32];
    field(line, n, b);
    return floatstr(b);
}

in(const line[], n)
{
    new b[32];
    field(line, n, b);
    return strval(b);
}

pl(const line[], n)
{
    new b[16];
    field(line, n, b);
    return player(b);
}

Run(seq, const line[])
{
    new cmd[24], a[1024], b[1024], c[128], d[128];
    field(line, 0, cmd);
    if (!strcmp(cmd, "msg"))
    {
        field(line, 1, c); field(line, 2, a); unescape(a);
        SendClientMessage(gLocal, hexval(c), "%s", a);
    }
    else if (!strcmp(cmd, "pmsg"))
    {
        field(line, 2, a); unescape(a);
        SendPlayerMessageToPlayer(gLocal, pl(line, 1), "%s", a);
    }
    else if (!strcmp(cmd, "dialog"))
    {
        field(line, 3, c); field(line, 4, a); field(line, 5, b); field(line, 6, d);
        unescape(a); unescape(c);
        new b2[64]; strmid(b2, d, 0, strlen(d));
        ShowPlayerDialog(gLocal, in(line, 1), DIALOG_STYLE:in(line, 2), c, "%s", b, b2, a);
    }
    else if (!strcmp(cmd, "gametext"))
    {
        field(line, 3, a); unescape(a);
        GameTextForPlayer(gLocal, "%s", in(line, 2), in(line, 1), a);
    }
    else if (!strcmp(cmd, "death")) SendDeathMessage(pl(line, 1), pl(line, 2), in(line, 3));
    else if (!strcmp(cmd, "pos"))
    {
        new p = pl(line, 1);
        SetPlayerWorldBounds(p, 20000.0, -20000.0, 20000.0, -20000.0);
        SetPlayerPos(p, fl(line, 2), fl(line, 3), fl(line, 4));
        SetPlayerFacingAngle(p, fl(line, 5));
        if (p == gLocal) SetCameraBehindPlayer(p);
    }
    else if (!strcmp(cmd, "hp")) SetPlayerHealth(pl(line, 1), fl(line, 2));
    else if (!strcmp(cmd, "armour")) SetPlayerArmour(pl(line, 1), fl(line, 2));
    else if (!strcmp(cmd, "color")) { field(line, 2, c); SetPlayerColor(pl(line, 1), hexval(c)); }
    else if (!strcmp(cmd, "score")) SetPlayerScore(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "bubble"))
    {
        field(line, 2, c); field(line, 5, a); unescape(a);
        SetPlayerChatBubble(pl(line, 1), "%s", hexval(c), fl(line, 3), in(line, 4), a);
    }
    else if (!strcmp(cmd, "veh"))
    {
        new rb[16], respawn = -1;
        if (field(line, 8, rb) && rb[0]) respawn = strval(rb);
        new v = CreateVehicle(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), in(line, 6), in(line, 7), respawn);
        printf("[ui-fs] %d veh-created %d model %d", seq, v, in(line, 1));
    }
    else if (!strcmp(cmd, "putin")) PutPlayerInVehicle(pl(line, 1), in(line, 2), in(line, 3));
    else if (!strcmp(cmd, "params"))
        SetVehicleParamsEx(in(line, 1), bool:in(line, 2), bool:in(line, 3), bool:in(line, 4), bool:in(line, 5), bool:in(line, 6), bool:in(line, 7), bool:in(line, 8));
    else if (!strcmp(cmd, "vhp")) SetVehicleHealth(in(line, 1), fl(line, 2));
    else if (!strcmp(cmd, "vdmg"))
        UpdateVehicleDamageStatus(in(line, 1), VEHICLE_PANEL_STATUS:in(line, 2), VEHICLE_DOOR_STATUS:in(line, 3), VEHICLE_LIGHT_STATUS:in(line, 4), VEHICLE_TYRE_STATUS:in(line, 5));
    else if (!strcmp(cmd, "vdel")) DestroyVehicle(in(line, 1));
    else if (!strcmp(cmd, "weapon")) GivePlayerWeapon(pl(line, 1), WEAPON:in(line, 2), in(line, 3));
    else if (!strcmp(cmd, "time")) SetPlayerTime(gLocal, in(line, 1), in(line, 2));
    else if (!strcmp(cmd, "weather")) SetPlayerWeather(gLocal, in(line, 1));
    else if (!strcmp(cmd, "interior")) SetPlayerInterior(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "camera")) SetCameraBehindPlayer(pl(line, 1));
    else if (!strcmp(cmd, "togglecontrol")) TogglePlayerControllable(pl(line, 1), bool:in(line, 2));
    else if (!strcmp(cmd, "spectate"))
    {
        TogglePlayerSpectating(pl(line, 1), true);
        PlayerSpectatePlayer(pl(line, 1), pl(line, 2));
    }
    else if (!strcmp(cmd, "attachtrailer")) AttachTrailerToVehicle(in(line, 1), in(line, 2));
    else if (!strcmp(cmd, "manual")) ManualVehicleEngineAndLights();
    else if (!strcmp(cmd, "stats"))
    {
        new p = pl(line, 1), Float:hp, Float:ar;
        GetPlayerHealth(p, hp); GetPlayerArmour(p, ar);
        printf("[ui-fs] %d stats %d health %.1f armour %.1f weapon %d", seq, p, hp, ar, _:GetPlayerWeapon(p));
    }
    else if (!strcmp(cmd, "where"))
    {
        new p = pl(line, 1), Float:x, Float:y, Float:z, Float:hp, Float:ang;
        GetPlayerPos(p, x, y, z); GetPlayerHealth(p, hp); GetPlayerFacingAngle(p, ang);
        printf("[ui-fs] %d where %d pos %.3f %.3f %.3f angle %.3f veh %d seat %d state %d health %.1f", seq, p, x, y, z, ang, GetPlayerVehicleID(p), GetPlayerVehicleSeat(p), _:GetPlayerState(p), hp);
    }
    else printf("[ui-fs] %d unknown command '%s'", seq, cmd);
    printf("[ui-fs] %d done %s", seq, cmd);
}

forward UiPoll();
public UiPoll()
{
    new File:f = fopen("uicmd.txt", io_read);
    if (!f) return 1;
    new line[2048];
    while (fread(f, line))
    {
        new n = strlen(line);
        while (n > 0 && (line[n - 1] == '\n' || line[n - 1] == '\r')) line[--n] = 0;
        new sp = strfind(line, " ");
        if (sp <= 0) continue;
        new num[12];
        strmid(num, line, 0, sp);
        new seq = strval(num);
        if (seq <= gLast) continue;
        gLast = seq;
        Run(seq, line[sp + 1]);
    }
    fclose(f);
    return 1;
}
