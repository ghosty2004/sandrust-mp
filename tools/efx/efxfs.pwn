// EFX-1 filterscript (trace lane only): fires exactly ONE server→client RPC per command, so the
// effect tracer (samp-tracer `efx`) sees the client's reaction to that RPC alone.
// tools/efx/scenario.sh appends lines "SEQ CMD|ARG|..." to scriptfiles/efxcmd.txt; every 100 ms
// this script runs the new ones and prints "[efx-fs] SEQ done CMD". Player = the traced player.
//   pos|x|y|z            SetPlayerPos            (RPC 12)
//   hp|value             SetPlayerHealth         (RPC 14)
//   skin|id              SetPlayerSkin           (RPC 153)
//   putin|veh|seat       PutPlayerInVehicle      (RPC 70)
//   veh|model|x|y|z|a    CreateVehicle (setup; prints "[efx-fs] SEQ veh ID")
//   where                prints position, health, skin, vehicle
// PAR-AUDIT additions (vehicle/object/pickup commands act on the last one created here):
//   posz|x|y|z SetPlayerPosFindZ (13)   angle|a SetPlayerFacingAngle (19)   ctrl|0/1 (15)
//   money|n GivePlayerMoney (18)        grav|g SetPlayerGravity (146)        int|i (156)
//   campos|x|y|z (157)  camlook|x|y|z (158)  cambehind (162)
//   vparams|engine|lights|alarm|doors|bonnet|boot|obj (24)  vhp|hp (147)  vpos|x|y|z (159)
//   vang|a (160)  putlast|seat (70, setup)  remove (71)  vehdel (165)
//   obj|model|x|y|z (44)  objmove|x|y|z|speed (99)  objstop (122)  objdel (47)
//   pickup|model|type|x|y|z (95)  pickdel (63)
//   dlg|id|style (61; id -1 hides)  gametext|text|ms|style (73)  tdshow (134)  tdhide (135)
#define FILTERSCRIPT
#pragma dynamic 16384
#include <open.mp>

new gLast = 0;
new gLocal = INVALID_PLAYER_ID;
new gLastVeh = INVALID_VEHICLE_ID;
new gLastObj = INVALID_OBJECT_ID;
new gLastPickup = -1;
new PlayerText:gTd = INVALID_PLAYER_TEXT_DRAW;

public OnFilterScriptInit()
{
    new File:f = fopen("efxcmd.txt", io_write);
    if (f) fclose(f);
    SetTimer("EfxPoll", 100, true);
    print("[efx-fs] loaded");
    return 1;
}

public OnPlayerConnect(playerid)
{
    if (gLocal == INVALID_PLAYER_ID) gLocal = playerid;
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    if (playerid == gLocal) gLocal = INVALID_PLAYER_ID;
    return 1;
}

public OnPlayerSpawn(playerid)
{
    SetPlayerWorldBounds(playerid, 20000.0, -20000.0, 20000.0, -20000.0);
    printf("[efx-fs] spawn %d", playerid);
    return 1;
}

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

Run(seq, const line[])
{
    new cmd[16], str[64];
    field(line, 0, cmd);
    new p = gLocal;
    if (!strcmp(cmd, "pos")) SetPlayerPos(p, fl(line, 1), fl(line, 2), fl(line, 3));
    else if (!strcmp(cmd, "hp")) SetPlayerHealth(p, fl(line, 1));
    else if (!strcmp(cmd, "skin")) SetPlayerSkin(p, in(line, 1));
    else if (!strcmp(cmd, "putin")) PutPlayerInVehicle(p, in(line, 1), in(line, 2));
    else if (!strcmp(cmd, "veh"))
    {
        new v = CreateVehicle(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), 1, 1, -1);
        gLastVeh = v;
        printf("[efx-fs] %d veh %d", seq, v);
    }
    else if (!strcmp(cmd, "posz")) SetPlayerPosFindZ(p, fl(line, 1), fl(line, 2), fl(line, 3));
    else if (!strcmp(cmd, "angle")) SetPlayerFacingAngle(p, fl(line, 1));
    else if (!strcmp(cmd, "ctrl")) TogglePlayerControllable(p, in(line, 1) != 0);
    else if (!strcmp(cmd, "money")) GivePlayerMoney(p, in(line, 1));
    else if (!strcmp(cmd, "grav")) SetPlayerGravity(p, fl(line, 1));
    else if (!strcmp(cmd, "int")) SetPlayerInterior(p, in(line, 1));
    else if (!strcmp(cmd, "campos")) SetPlayerCameraPos(p, fl(line, 1), fl(line, 2), fl(line, 3));
    else if (!strcmp(cmd, "camlook")) SetPlayerCameraLookAt(p, fl(line, 1), fl(line, 2), fl(line, 3));
    else if (!strcmp(cmd, "cambehind")) SetCameraBehindPlayer(p);
    else if (!strcmp(cmd, "vparams")) SetVehicleParamsEx(gLastVeh, in(line, 1), in(line, 2), in(line, 3), in(line, 4),
        in(line, 5), in(line, 6), in(line, 7));
    else if (!strcmp(cmd, "vhp")) SetVehicleHealth(gLastVeh, fl(line, 1));
    else if (!strcmp(cmd, "vpos")) SetVehiclePos(gLastVeh, fl(line, 1), fl(line, 2), fl(line, 3));
    else if (!strcmp(cmd, "vang")) SetVehicleZAngle(gLastVeh, fl(line, 1));
    else if (!strcmp(cmd, "putlast")) PutPlayerInVehicle(p, gLastVeh, in(line, 1));
    else if (!strcmp(cmd, "remove")) RemovePlayerFromVehicle(p);
    else if (!strcmp(cmd, "vehdel")) DestroyVehicle(gLastVeh);
    else if (!strcmp(cmd, "obj"))
    {
        gLastObj = CreateObject(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), 0.0, 0.0, 0.0);
        printf("[efx-fs] %d obj %d", seq, gLastObj);
    }
    else if (!strcmp(cmd, "objmove")) MoveObject(gLastObj, fl(line, 1), fl(line, 2), fl(line, 3), fl(line, 4));
    else if (!strcmp(cmd, "objstop")) StopObject(gLastObj);
    else if (!strcmp(cmd, "objdel")) DestroyObject(gLastObj);
    else if (!strcmp(cmd, "pickup"))
    {
        gLastPickup = CreatePickup(in(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), -1);
        printf("[efx-fs] %d pickup %d", seq, gLastPickup);
    }
    else if (!strcmp(cmd, "pickdel")) DestroyPickup(gLastPickup);
    else if (!strcmp(cmd, "dlg"))
        ShowPlayerDialog(p, in(line, 1), DIALOG_STYLE:in(line, 2), "EFX", "EFX dialog body", "OK", "Cancel");
    else if (!strcmp(cmd, "gametext"))
    {
        field(line, 1, str);
        GameTextForPlayer(p, str, in(line, 2), in(line, 3));
    }
    else if (!strcmp(cmd, "tdshow"))
    {
        if (gTd == INVALID_PLAYER_TEXT_DRAW) gTd = CreatePlayerTextDraw(p, 320.0, 240.0, "EFX");
        PlayerTextDrawShow(p, gTd);
    }
    else if (!strcmp(cmd, "tdhide")) PlayerTextDrawHide(p, gTd);
    else if (!strcmp(cmd, "where"))
    {
        new Float:x, Float:y, Float:z, Float:hp;
        GetPlayerPos(p, x, y, z); GetPlayerHealth(p, hp);
        printf("[efx-fs] %d where pos %.3f %.3f %.3f health %.1f skin %d veh %d seat %d state %d", seq, x, y, z, hp,
            GetPlayerSkin(p), GetPlayerVehicleID(p), GetPlayerVehicleSeat(p), _:GetPlayerState(p));
    }
    else printf("[efx-fs] %d unknown command '%s'", seq, cmd);
    printf("[efx-fs] %d done %s", seq, cmd);
}

forward EfxPoll();
public EfxPoll()
{
    new File:f = fopen("efxcmd.txt", io_read);
    if (!f) return 1;
    new line[256];
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
