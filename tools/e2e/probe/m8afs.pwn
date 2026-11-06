// M8a filterscript: a superset of tools/trace/uifs.pwn (same command file protocol and
// log lines, so one command sequence runs on the original client and on ours).
// Extra commands:
//   td|ID|x|y|font|align|text         global textdraw slot ID (0..9): letter 0.4×1.6, box,
//                                     text size per alignment, shown to L
//   tdsel|ID|0/1   tdset|ID|text   tdhide|ID
//   select|RRGGBBAA   cancelsel      SelectTextDraw / CancelSelectTextDraw (L)
//   menu|title|x|y|w1|w2|item;item…  CreateMenu (1 or 2 columns: w2 > 0) + items
//   menushow   menuhide             ShowMenuForPlayer / HideMenuForPlayer (L)
//   tdsize|ID|x|y   tdmodel|ID|model    TextDrawTextSize / model preview (font 5), re-shown
// Original header:
// TRACE-3/TRACE-5 filterscript (trace lane only): server-side actions on demand.
// tools/trace/ui.sh appends lines "SEQ CMD|ARG|ARG|..." to scriptfiles/uicmd.txt; every
// 100 ms this script runs the lines with a SEQ above the last one it ran and prints
// "[ui-fs] SEQ done ..." (the scenario waits for that line). In texts, "\n" and "\t" are
// turned into newline / tab. Players: "L" = the traced (first non-bot) player, "B1".."B4" =
// bots in join order, a number = that id, "-" = INVALID_PLAYER_ID.
//   msg|RRGGBBAA|text                      SendClientMessage(L)
//   pmsg|SENDER|text                       SendPlayerMessageToPlayer(L, sender)
//   dialog|ID|STYLE|caption|body|b1|b2     ShowPlayerDialog(L)
//   chatdlg|ID|STYLE|DELAYMS|VIA           INPUT-1: the next chat line answers with ShowPlayerDialog(ID, STYLE)
//                                          (VIA 0 = a command "/dlg…" in OnPlayerCommandText, 1 = plain text in
//                                          OnPlayerText; DELAYMS 0 = inside the callback, else a timer); ID -1 = off
//   chaindlg|FROM|TO|STYLE                 INPUT-1: OnDialogResponse for FROM shows dialog TO (FROM -1 = off)
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
//   surf|P                                 prints GetPlayerSurfingVehicleID / GetPlayerSurfingObjectID
//   manual                                 ManualVehicleEngineAndLights()
//   stats|P                                prints health, armour, weapon
// TRACE-6 (world entities, combat); ids are printed as "[ui-fs] SEQ <what> ID":
//   obj|model|x|y|z|rx|ry|rz|dd            CreateObject          objdel|ID  objstop|ID
//   objmove|ID|x|y|z|speed[|rx|ry|rz]      MoveObject            objpos|ID|x|y|z  objrot|ID|rx|ry|rz
//   objmat|ID|index|model|txd|tex|AARRGGBB SetObjectMaterial     nocamcol|ID
//   objtext|ID|index|text|size|font|fsize|bold|AARRGGBB|AARRGGBB|align   SetObjectMaterialText
//   mtscene|x|y|z   mtmat                  material-text parameter set (F4 element mattext): MtScene below
//   objattveh|ID|VEH|ox|oy|oz|rx|ry|rz     objattobj|ID|PARENT|ox|oy|oz|rx|ry|rz|sync  objattpl|ID|P|ox|oy|oz|rx|ry|rz
//   objedit|P|ID  objsel|P  editcancel|P   EditObject / SelectObject / CancelEdit
//   att|P|slot|model|bone|ox|oy|oz|rx|ry|rz|sx|sy|sz   SetPlayerAttachedObject   attdel|P|slot  attedit|P|slot
//   pickup|model|type|x|y|z  pickdel|ID    CreatePickup / DestroyPickup
//   actor|skin|x|y|z|angle  actdel|ID  actinv|ID|0/1  acthp|ID|hp  actpos|ID|x|y|z  actface|ID|a
//   actanim|ID|lib|name|delta|loop|lx|ly|freeze|time   actclear|ID
//   cp|P|x|y|z|size  cpoff|P  rcp|P|type|x|y|z|nx|ny|nz|size  rcpoff|P
//   gz|minx|miny|maxx|maxy (GangZoneCreate)  gzshow|P|ID|RRGGBBAA  gzflash|P|ID|RRGGBBAA  gzstop|P|ID  gzhide|P|ID
//   icon|P|iconid|x|y|z|type|RRGGBBAA-or-decimal|style   icondel|P|iconid
//   label|text|RRGGBBAA|x|y|z|dist|los     Create3DTextLabel     labeldel|ID
//   labelpl|ID|P|ox|oy|oz  labelveh|ID|VEH|ox|oy|oz           Attach3DTextLabelToPlayer / ToVehicle
//   plabel|P|text|RRGGBBAA|x|y|z|dist|attplayer|attveh|los    CreatePlayer3DTextLabel
//   rmb|P|model|x|y|z|radius               RemoveBuildingForPlayer
//   team|P|team  skill|P|skill|level  explode|x|y|z|type|radius  armed|P|weapon  ammo|P|weapon|ammo
//   resetw|P  forceclass|P  special|P|action  kill|P (health 0)
#define FILTERSCRIPT
#pragma dynamic 65536
#include <open.mp>

new gLast = 0;
new gBots[8] = { INVALID_PLAYER_ID, ... };
new gNumBots = 0;
new gLocal = INVALID_PLAYER_ID;
// INPUT-1 (chatdlg / chaindlg)
new gCdId = -1, gCdStyle, gCdDelay, gCdVia;
new gChFrom = -1, gChTo, gChStyle;
new Text:gTd[10] = { Text:INVALID_TEXT_DRAW, ... };
new Menu:gMenu = Menu:INVALID_MENU;

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

public OnPlayerClickTextDraw(playerid, Text:clickedid)
{
    printf("[ui-fs] click-textdraw player %d id %d", playerid, _:clickedid);
    return 1;
}

public OnPlayerSelectedMenuRow(playerid, row)
{
    printf("[ui-fs] menu-row player %d row %d", playerid, row);
    return 1;
}

public OnPlayerExitedMenu(playerid)
{
    printf("[ui-fs] menu-exit player %d", playerid);
    return 1;
}

public OnDialogResponse(playerid, dialogid, response, listitem, inputtext[])
{
    printf("[ui-fs] dialog-response player %d id %d response %d listitem %d text '%s'", playerid, dialogid, response, listitem, inputtext);
    if (dialogid == gChFrom && gChFrom != -1)
    {
        ShowPlayerDialog(playerid, gChTo, DIALOG_STYLE:gChStyle, "Chained", "Second dialog:", "OK", "Cancel");
        printf("[ui-fs] chain-shown %d", gChTo);
    }
    return 1;
}

public OnPlayerText(playerid, text[])
{
    printf("[ui-fs] text %d '%s'", playerid, text);
    if (gCdId != -1 && gCdVia == 1) ChatDialog(playerid);
    return 1;
}

public OnPlayerCommandText(playerid, cmdtext[])
{
    printf("[ui-fs] command %d '%s'", playerid, cmdtext);
    if (gCdId != -1 && gCdVia == 0 && !strcmp(cmdtext, "/dlg", false, 4))
    {
        ChatDialog(playerid);
        return 1;
    }
    return 0;
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[ui-fs] state %d %d->%d veh %d seat %d", playerid, _:oldstate, _:newstate, GetPlayerVehicleID(playerid), GetPlayerVehicleSeat(playerid));
    return 1;
}

public OnPlayerInteriorChange(playerid, newinteriorid, oldinteriorid)
{
    printf("[ui-fs] interior %d %d->%d", playerid, oldinteriorid, newinteriorid);
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

public OnPlayerPickUpPickup(playerid, pickupid)
{
    printf("[ui-fs] pickup-collected %d pickup %d", playerid, pickupid);
    return 1;
}

public OnPlayerEnterCheckpoint(playerid) { printf("[ui-fs] cp-enter %d", playerid); return 1; }
public OnPlayerLeaveCheckpoint(playerid) { printf("[ui-fs] cp-leave %d", playerid); return 1; }
public OnPlayerEnterRaceCheckpoint(playerid) { printf("[ui-fs] rcp-enter %d", playerid); return 1; }
public OnPlayerLeaveRaceCheckpoint(playerid) { printf("[ui-fs] rcp-leave %d", playerid); return 1; }

public OnPlayerWeaponShot(playerid, WEAPON:weaponid, BULLET_HIT_TYPE:hittype, hitid, Float:fX, Float:fY, Float:fZ)
{
    new Float:ox, Float:oy, Float:oz, Float:hx, Float:hy, Float:hz;
    GetPlayerLastShotVectors(playerid, ox, oy, oz, hx, hy, hz);
    printf("[ui-fs] shot %d weapon %d hittype %d hitid %d offset %.4f %.4f %.4f origin %.3f %.3f %.3f hitpos %.3f %.3f %.3f",
        playerid, _:weaponid, _:hittype, hitid, fX, fY, fZ, ox, oy, oz, hx, hy, hz);
    return 1;
}

public OnPlayerGiveDamageActor(playerid, damaged_actorid, Float:amount, WEAPON:weaponid, bodypart)
{
    new Float:hp;
    GetActorHealth(damaged_actorid, hp);
    printf("[ui-fs] actor-damage %d actor %d %.4f weapon %d part %d actor-health %.2f", playerid, damaged_actorid, amount, _:weaponid, bodypart, hp);
    return 1;
}

public OnActorStreamIn(actorid, forplayerid)
{
    printf("[ui-fs] actor-streamin %d for %d", actorid, forplayerid);
    return 1;
}

public OnPlayerSelectObject(playerid, SELECT_OBJECT:type, objectid, modelid, Float:fX, Float:fY, Float:fZ)
{
    printf("[ui-fs] select-object %d type %d obj %d model %d pos %.3f %.3f %.3f", playerid, _:type, objectid, modelid, fX, fY, fZ);
    return 1;
}

public OnPlayerEditObject(playerid, playerobject, objectid, EDIT_RESPONSE:response, Float:fX, Float:fY, Float:fZ, Float:fRotX, Float:fRotY, Float:fRotZ)
{
    printf("[ui-fs] edit-object %d playerobj %d obj %d response %d pos %.3f %.3f %.3f rot %.3f %.3f %.3f", playerid, playerobject, objectid, _:response, fX, fY, fZ, fRotX, fRotY, fRotZ);
    return 1;
}

public OnPlayerEditAttachedObject(playerid, EDIT_RESPONSE:response, index, modelid, boneid, Float:fOffsetX, Float:fOffsetY, Float:fOffsetZ, Float:fRotX, Float:fRotY, Float:fRotZ, Float:fScaleX, Float:fScaleY, Float:fScaleZ)
{
    printf("[ui-fs] edit-attached %d response %d index %d model %d bone %d off %.3f %.3f %.3f rot %.3f %.3f %.3f scale %.3f %.3f %.3f", playerid, _:response, index, modelid, boneid, fOffsetX, fOffsetY, fOffsetZ, fRotX, fRotY, fRotZ, fScaleX, fScaleY, fScaleZ);
    return 1;
}

public OnObjectMoved(objectid)
{
    printf("[ui-fs] object-moved %d tick %d", objectid, GetTickCount());
    return 1;
}

public OnPlayerClickMap(playerid, Float:fX, Float:fY, Float:fZ)
{
    printf("[ui-fs] click-map %d %.2f %.2f %.2f", playerid, fX, fY, fZ);
    return 1;
}

public OnPlayerRequestSpawn(playerid)
{
    printf("[ui-fs] request-spawn %d", playerid);
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

// a colour field: hex (RRGGBBAA) or, with a leading '#', decimal (e.g. a player id for map icons)
colour(const line[], n)
{
    new b[32];
    field(line, n, b);
    if (b[0] == '#') return strval(b[1]);
    return hexval(b);
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

// One material-text object of the F4 mattext set: object `n` of a 4-column grid on the plane x = bx (columns
// 4.5 m apart along y, rows 4 m apart in z), turned by rz.
MtObj(n, model, Float:bx, Float:by, Float:bz, Float:rz)
{
    new Float:y = by + float(n % 4) * 4.5, Float:z = bz + float(n / 4) * 4.0;
    // Objects 6.. are player objects: open.mp re-creates a global object (RPC 44) on every material change but
    // sends RPC 84 for player objects.
    if (n >= 6) return CreatePlayerObject(gLocal, model, bx, y, z, 0.0, 0.0, rz, 300.0);
    return CreateObject(model, bx, y, z, 0.0, 0.0, rz, 300.0);
}

// F4 element mattext (tools/diff/visual-scenario.sh visual_mattext): SetObjectMaterialText over sizes, fonts,
// bold, alignment, background colours (0 = transparent, opaque, half alpha), colour codes and newlines, and
// SetObjectMaterial + text on the same and on different material indices. `mtscene` creates all 14 objects and
// gives the global objects 0..5 their text (open.mp re-creates a global object with RPC 44 on every material
// change); `mtmat` (a later command) sets the rest on existing player objects 6..13 (RPC 84).
new gMt[14];
MtScene(seq, Float:bx, Float:by, Float:bz)
{
    gMt[0] = MtObj(0, 19353, bx, by, bz, 0.0);
    SetObjectMaterialText(gMt[0], "{0000FF}Romania {FFFF00}Super {FF0000}Stunt", 0, OBJECT_MATERIAL_SIZE_512x256, "Arial", 60, true, 0xFFFFFFFF, 0x00000000, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    gMt[1] = MtObj(1, 19353, bx, by, bz, 0.0);
    SetObjectMaterialText(gMt[1], "left top\nsecond {00FF00}line", 0, OBJECT_MATERIAL_SIZE_256x128, "Arial", 24, false, 0xFFFFFFFF, 0xFF000000, OBJECT_MATERIAL_TEXT_ALIGN_LEFT);
    gMt[2] = MtObj(2, 19353, bx, by, bz, 0.0);
    SetObjectMaterialText(gMt[2], "right {FF0000}red", 0, OBJECT_MATERIAL_SIZE_128x64, "Verdana", 20, true, 0xFF000000, 0xFFFFFF00, OBJECT_MATERIAL_TEXT_ALIGN_RIGHT);
    gMt[3] = MtObj(3, 19482, bx, by, bz, 0.0);
    SetObjectMaterialText(gMt[3], "BIG\nTWO", 0, OBJECT_MATERIAL_SIZE_512x512, "Impact", 80, false, 0xFFFFFF00, 0x80FF0000, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    gMt[4] = MtObj(4, 18244, bx, by, bz, 0.0);
    SetObjectMaterialText(gMt[4], "transparent left", 0, OBJECT_MATERIAL_SIZE_512x128, "Arial", 40, true, 0xFF00FF00, 0x00000000, OBJECT_MATERIAL_TEXT_ALIGN_LEFT);
    gMt[5] = MtObj(5, 19327, bx, by, bz, 0.0);
    SetObjectMaterialText(gMt[5], "x", 0, OBJECT_MATERIAL_SIZE_32x32, "Arial", 20, false, 0xFFFFFFFF, 0xFF0000FF, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    gMt[6] = MtObj(6, 19353, bx, by, bz, 0.0);
    gMt[7] = MtObj(7, 19353, bx, by, bz, 0.0);
    gMt[8] = MtObj(8, 19353, bx, by, bz, 0.0);
    gMt[9] = MtObj(9, 19353, bx, by, bz, 0.0);
    gMt[10] = MtObj(10, 19353, bx, by, bz, 0.0);
    gMt[11] = MtObj(11, 19353, bx, by, bz, 0.0);
    gMt[12] = MtObj(12, 7914, bx, by, bz, 0.0);
    gMt[13] = MtObj(13, 7914, bx, by, bz, 0.0);
    printf("[ui-fs] %d mtscene %d..%d", seq, gMt[0], gMt[13]);
}

// The second half of the set: material records for objects that already exist (RPC 84).
MtMat(seq)
{
    // Texture, then text on the same index (the text replaces the texture).
    SetPlayerObjectMaterial(gLocal, gMt[6], 0, 18646, "matcolours", "white", 0xFFFFFFFF);
    SetPlayerObjectMaterialText(gLocal, gMt[6], "mat {FF0000}+ text", 0, OBJECT_MATERIAL_SIZE_256x256, "Arial", 50, true, 0xFF000000, 0x00000000, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    // Text, then texture on the same index (the text is not shown any more).
    SetPlayerObjectMaterialText(gLocal, gMt[7], "hidden", 0, OBJECT_MATERIAL_SIZE_256x256, "Arial", 50, true, 0xFFFFFFFF, 0xFF00FF00, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    SetPlayerObjectMaterial(gLocal, gMt[7], 0, 18646, "matcolours", "red", 0xFFFFFFFF);
    // Text larger than its target (NOCLIP), tiny sizes, other fonts.
    SetPlayerObjectMaterialText(gLocal, gMt[8], "clip", 0, OBJECT_MATERIAL_SIZE_64x32, "Arial", 80, false, 0xFFFFFFFF, 0xFF202020, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    SetPlayerObjectMaterialText(gLocal, gMt[9], "right\ntransparent", 0, OBJECT_MATERIAL_SIZE_256x32, "Courier New", 30, false, 0xFF00FFFF, 0x00000000, OBJECT_MATERIAL_TEXT_ALIGN_RIGHT);
    SetPlayerObjectMaterialText(gLocal, gMt[10], "{FFFFFF}white {000000}black", 0, OBJECT_MATERIAL_SIZE_512x64, "Tahoma", 40, true, 0xFFFF0000, 0xFF808080, OBJECT_MATERIAL_TEXT_ALIGN_LEFT);
    SetPlayerObjectMaterialText(gLocal, gMt[11], "30", 0, OBJECT_MATERIAL_SIZE_64x64, "Arial", 30, true, 0xFFFFFFFF, 0xFF0000FF, OBJECT_MATERIAL_TEXT_ALIGN_LEFT);
    // Texture on index 0 + text on index 1, and text on 0 + texture on 1 (billboard with several materials).
    SetPlayerObjectMaterial(gLocal, gMt[12], 0, 18646, "matcolours", "blue", 0xFFFFFFFF);
    SetPlayerObjectMaterialText(gLocal, gMt[12], "index {FFFF00}one", 1, OBJECT_MATERIAL_SIZE_256x128, "Arial", 40, true, 0xFFFFFFFF, 0x00000000, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    SetPlayerObjectMaterialText(gLocal, gMt[13], "index zero", 0, OBJECT_MATERIAL_SIZE_128x128, "Arial", 30, false, 0xFFFFFFFF, 0xFF0000FF, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    SetPlayerObjectMaterial(gLocal, gMt[13], 1, 18646, "matcolours", "green", 0xFFFFFFFF);
    printf("[ui-fs] %d mtmat", seq);
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
    else if (!strcmp(cmd, "chatdlg"))
    {
        gCdId = in(line, 1); gCdStyle = in(line, 2); gCdDelay = in(line, 3); gCdVia = in(line, 4);
    }
    else if (!strcmp(cmd, "chaindlg"))
    {
        gChFrom = in(line, 1); gChTo = in(line, 2); gChStyle = in(line, 3);
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
    else if (!strcmp(cmd, "campos")) SetPlayerCameraPos(pl(line, 1), fl(line, 2), fl(line, 3), fl(line, 4));
    else if (!strcmp(cmd, "camlook")) SetPlayerCameraLookAt(pl(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), CAMERA_CUT);
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
    else if (!strcmp(cmd, "surf"))
    {
        new p = pl(line, 1);
        printf("[ui-fs] %d surf %d vehicle %d object %d", seq, p, GetPlayerSurfingVehicleID(p), GetPlayerSurfingObjectID(p));
    }
    else if (!strcmp(cmd, "obj"))
        printf("[ui-fs] %d object %d", seq, CreateObject(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), fl(line, 8)));
    else if (!strcmp(cmd, "objdel")) DestroyObject(in(line, 1));
    else if (!strcmp(cmd, "objstop")) StopObject(in(line, 1));
    else if (!strcmp(cmd, "objmove"))
    {
        new rb[16], Float:rx = -1000.0, Float:ry = -1000.0, Float:rz = -1000.0;
        if (field(line, 6, rb) && rb[0]) { rx = fl(line, 6); ry = fl(line, 7); rz = fl(line, 8); }
        printf("[ui-fs] %d move-time %d tick %d", seq, MoveObject(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), rx, ry, rz), GetTickCount());
    }
    else if (!strcmp(cmd, "objpos")) SetObjectPos(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4));
    else if (!strcmp(cmd, "objrot")) SetObjectRot(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4));
    else if (!strcmp(cmd, "objmat"))
    {
        field(line, 4, c); field(line, 5, d);
        SetObjectMaterial(in(line, 1), in(line, 2), in(line, 3), c, d, colour(line, 6));
    }
    else if (!strcmp(cmd, "objtext"))
    {
        field(line, 3, a); unescape(a); field(line, 5, c);
        SetObjectMaterialText(in(line, 1), a, in(line, 2), OBJECT_MATERIAL_SIZE:in(line, 4), c, in(line, 6), bool:in(line, 7), colour(line, 8), colour(line, 9), OBJECT_MATERIAL_TEXT_ALIGN:in(line, 10));
    }
    else if (!strcmp(cmd, "mtscene")) MtScene(seq, fl(line, 1), fl(line, 2), fl(line, 3));
    else if (!strcmp(cmd, "mtmat")) MtMat(seq);
    else if (!strcmp(cmd, "nocamcol")) SetObjectNoCameraCol(in(line, 1));
    else if (!strcmp(cmd, "objattveh")) AttachObjectToVehicle(in(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), fl(line, 8));
    else if (!strcmp(cmd, "objattobj")) AttachObjectToObject(in(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), fl(line, 8), bool:in(line, 9));
    else if (!strcmp(cmd, "objattpl")) AttachObjectToPlayer(in(line, 1), pl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), fl(line, 8));
    else if (!strcmp(cmd, "objedit")) EditObject(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "objsel")) SelectObject(pl(line, 1));
    else if (!strcmp(cmd, "editcancel")) CancelEdit(pl(line, 1));
    else if (!strcmp(cmd, "att"))
        SetPlayerAttachedObject(pl(line, 1), in(line, 2), in(line, 3), in(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), fl(line, 8), fl(line, 9), fl(line, 10), fl(line, 11), fl(line, 12), fl(line, 13));
    else if (!strcmp(cmd, "attdel")) RemovePlayerAttachedObject(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "attedit")) EditAttachedObject(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "pickup"))
        printf("[ui-fs] %d pickup %d", seq, CreatePickup(in(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5)));
    else if (!strcmp(cmd, "pickdel")) DestroyPickup(in(line, 1));
    else if (!strcmp(cmd, "actor"))
        printf("[ui-fs] %d actor %d", seq, CreateActor(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5)));
    else if (!strcmp(cmd, "actdel")) DestroyActor(in(line, 1));
    else if (!strcmp(cmd, "actinv")) SetActorInvulnerable(in(line, 1), bool:in(line, 2));
    else if (!strcmp(cmd, "acthp")) SetActorHealth(in(line, 1), fl(line, 2));
    else if (!strcmp(cmd, "actpos")) SetActorPos(in(line, 1), fl(line, 2), fl(line, 3), fl(line, 4));
    else if (!strcmp(cmd, "actface")) SetActorFacingAngle(in(line, 1), fl(line, 2));
    else if (!strcmp(cmd, "actanim"))
    {
        field(line, 2, c); field(line, 3, d);
        ApplyActorAnimation(in(line, 1), c, d, fl(line, 4), bool:in(line, 5), bool:in(line, 6), bool:in(line, 7), bool:in(line, 8), in(line, 9));
    }
    else if (!strcmp(cmd, "actclear")) ClearActorAnimations(in(line, 1));
    else if (!strcmp(cmd, "cp")) SetPlayerCheckpoint(pl(line, 1), fl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5));
    else if (!strcmp(cmd, "cpoff")) DisablePlayerCheckpoint(pl(line, 1));
    else if (!strcmp(cmd, "rcp"))
        SetPlayerRaceCheckpoint(pl(line, 1), CP_TYPE:in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), fl(line, 8), fl(line, 9));
    else if (!strcmp(cmd, "rcpoff")) DisablePlayerRaceCheckpoint(pl(line, 1));
    else if (!strcmp(cmd, "gz"))
        printf("[ui-fs] %d gangzone %d", seq, GangZoneCreate(fl(line, 1), fl(line, 2), fl(line, 3), fl(line, 4)));
    else if (!strcmp(cmd, "gzshow")) GangZoneShowForPlayer(pl(line, 1), in(line, 2), colour(line, 3));
    else if (!strcmp(cmd, "gzflash")) GangZoneFlashForPlayer(pl(line, 1), in(line, 2), colour(line, 3));
    else if (!strcmp(cmd, "gzstop")) GangZoneStopFlashForPlayer(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "gzhide")) GangZoneHideForPlayer(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "icon"))
        SetPlayerMapIcon(pl(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), in(line, 6), colour(line, 7), MAPICON:in(line, 8));
    else if (!strcmp(cmd, "icondel")) RemovePlayerMapIcon(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "label"))
    {
        field(line, 1, a); unescape(a);
        printf("[ui-fs] %d label %d", seq, _:Create3DTextLabel(a, colour(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), 0, bool:in(line, 7)));
    }
    else if (!strcmp(cmd, "labeldel")) Delete3DTextLabel(Text3D:in(line, 1));
    else if (!strcmp(cmd, "labelpl")) Attach3DTextLabelToPlayer(Text3D:in(line, 1), pl(line, 2), fl(line, 3), fl(line, 4), fl(line, 5));
    else if (!strcmp(cmd, "labelveh")) Attach3DTextLabelToVehicle(Text3D:in(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5));
    else if (!strcmp(cmd, "plabel"))
    {
        field(line, 2, a); unescape(a);
        new ap[16], ap_id = INVALID_PLAYER_ID, av = INVALID_VEHICLE_ID;
        field(line, 8, ap); if (ap[0] && ap[0] != '-') ap_id = player(ap);
        field(line, 9, ap); if (ap[0] && ap[0] != '-') av = strval(ap);
        printf("[ui-fs] %d plabel %d", seq, _:CreatePlayer3DTextLabel(pl(line, 1), a, colour(line, 3), fl(line, 4), fl(line, 5), fl(line, 6), fl(line, 7), ap_id, av, bool:in(line, 10)));
    }
    else if (!strcmp(cmd, "rmb")) RemoveBuildingForPlayer(pl(line, 1), in(line, 2), fl(line, 3), fl(line, 4), fl(line, 5), fl(line, 6));
    else if (!strcmp(cmd, "team")) SetPlayerTeam(pl(line, 1), in(line, 2));
    else if (!strcmp(cmd, "skill")) SetPlayerSkillLevel(pl(line, 1), WEAPONSKILL:in(line, 2), in(line, 3));
    else if (!strcmp(cmd, "explode")) CreateExplosion(fl(line, 1), fl(line, 2), fl(line, 3), in(line, 4), fl(line, 5));
    else if (!strcmp(cmd, "armed")) SetPlayerArmedWeapon(pl(line, 1), WEAPON:in(line, 2));
    else if (!strcmp(cmd, "ammo")) SetPlayerAmmo(pl(line, 1), WEAPON:in(line, 2), in(line, 3));
    else if (!strcmp(cmd, "resetw")) ResetPlayerWeapons(pl(line, 1));
    else if (!strcmp(cmd, "forceclass")) ForceClassSelection(pl(line, 1));
    else if (!strcmp(cmd, "special")) SetPlayerSpecialAction(pl(line, 1), SPECIAL_ACTION:in(line, 2));
    else if (!strcmp(cmd, "kill")) SetPlayerHealth(pl(line, 1), 0.0);
    else if (!strcmp(cmd, "td"))
    {
        new id = in(line, 1);
        field(line, 6, a); unescape(a);
        if (gTd[id] != Text:INVALID_TEXT_DRAW) TextDrawDestroy(gTd[id]);
        gTd[id] = TextDrawCreate(fl(line, 2), fl(line, 3), "%s", a);
        TextDrawLetterSize(gTd[id], 0.4, 1.6);
        TextDrawFont(gTd[id], TEXT_DRAW_FONT:in(line, 4));
        TextDrawAlignment(gTd[id], TEXT_DRAW_ALIGN:in(line, 5));
        TextDrawColour(gTd[id], 0xFFFFFFFF);
        TextDrawUseBox(gTd[id], true);
        TextDrawBoxColour(gTd[id], 0x00000088);
        TextDrawSetShadow(gTd[id], 1);
        TextDrawSetOutline(gTd[id], 0);
        TextDrawBackgroundColour(gTd[id], 0x000000FF);
        TextDrawSetProportional(gTd[id], true);
        if (in(line, 5) == 2) TextDrawTextSize(gTd[id], 20.0, 200.0);
        else TextDrawTextSize(gTd[id], fl(line, 2) + 200.0, 20.0);
        TextDrawShowForPlayer(gLocal, gTd[id]);
        printf("[ui-fs] %d td %d = %d", seq, id, _:gTd[id]);
    }
    else if (!strcmp(cmd, "tdsel"))
    {
        new id = in(line, 1);
        TextDrawSetSelectable(gTd[id], bool:in(line, 2));
        TextDrawShowForPlayer(gLocal, gTd[id]);
    }
    else if (!strcmp(cmd, "tdset")) { field(line, 2, a); unescape(a); TextDrawSetString(gTd[in(line, 1)], "%s", a); }
    else if (!strcmp(cmd, "tdsize"))
    {
        TextDrawTextSize(gTd[in(line, 1)], fl(line, 2), fl(line, 3));
        TextDrawShowForPlayer(gLocal, gTd[in(line, 1)]);
    }
    else if (!strcmp(cmd, "tdmodel"))
    {
        new id = in(line, 1);
        TextDrawFont(gTd[id], TEXT_DRAW_FONT_MODEL_PREVIEW);
        TextDrawSetPreviewModel(gTd[id], in(line, 2));
        TextDrawSetPreviewRot(gTd[id], -16.0, 0.0, -55.0, 1.0);
        TextDrawBackgroundColour(gTd[id], 0x00000044);
        TextDrawShowForPlayer(gLocal, gTd[id]);
    }
    else if (!strcmp(cmd, "tdhide")) TextDrawHideForPlayer(gLocal, gTd[in(line, 1)]);
    else if (!strcmp(cmd, "select")) { field(line, 1, c); SelectTextDraw(gLocal, hexval(c)); }
    else if (!strcmp(cmd, "cancelsel")) CancelSelectTextDraw(gLocal);
    else if (!strcmp(cmd, "menu"))
    {
        field(line, 1, c);
        new Float:w2 = fl(line, 5);
        if (gMenu != Menu:INVALID_MENU) DestroyMenu(gMenu);
        gMenu = CreateMenu(c, w2 > 0.0 ? 2 : 1, fl(line, 2), fl(line, 3), fl(line, 4), w2);
        field(line, 6, a);
        new item[64], start = 0, n = 0;
        for (new i = 0; ; i++)
        {
            if (a[i] == ';' || a[i] == 0)
            {
                strmid(item, a, start, i);
                AddMenuItem(gMenu, 0, item);
                n++;
                if (a[i] == 0) break;
                start = i + 1;
            }
        }
        if (w2 > 0.0) { AddMenuItem(gMenu, 1, "$1"); SetMenuColumnHeader(gMenu, 0, "Item"); }
        printf("[ui-fs] %d menu %d items %d", seq, _:gMenu, n);
    }
    else if (!strcmp(cmd, "menushow")) ShowMenuForPlayer(gMenu, gLocal);
    else if (!strcmp(cmd, "menuhide")) HideMenuForPlayer(gMenu, gLocal);
    else printf("[ui-fs] %d unknown command '%s'", seq, cmd);
    printf("[ui-fs] %d done %s", seq, cmd);
}

ChatDialog(playerid)
{
    if (gCdDelay <= 0) ChatDialogShow(playerid);
    else SetTimerEx("ChatDialogShow", gCdDelay, false, "i", playerid);
}

forward ChatDialogShow(playerid);
public ChatDialogShow(playerid)
{
    ShowPlayerDialog(playerid, gCdId, DIALOG_STYLE:gCdStyle, "Chat dialog", "Type here:", "OK", "Cancel");
    printf("[ui-fs] chat-dialog-shown %d", gCdId);
    return 1;
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
