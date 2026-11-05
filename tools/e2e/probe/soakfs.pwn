// Soak filterscript (tools/e2e/soak.sh, PLAN F7): the server's view of every player and
// vehicle on demand, plus the scripted server-side actions of the soak.
// soak.sh (via soak_mon.py) appends lines "SEQ CMD|ARG|..." to scriptfiles/soakcmd.txt;
// every 100 ms this script runs the lines with a SEQ above the last one it ran and prints
// "[soak] SEQ done CMD". Commands:
//   dump|NAME              one snapshot: "[soak] SEQ dump begin local=ID tick=T", one "p" line per
//                          connected player, one "v" line per vehicle, "[soak] SEQ dump end"
//                          (streamed = streamed in for player NAME, the client under test)
//   kick|NAME              Kick the player called NAME
//   rcon|TEXT              SendRconCommand (e.g. gmx)
//   vehspot|model|x|y|z|a|n  keep n vehicles of model parked in a row at x,y,z (recreated after
//                          gmx or destruction, checked every 5 s)
// The command file is truncated on load, so a gmx that reloads this script starts clean.
#define FILTERSCRIPT
#pragma dynamic 32768
#include <open.mp>

new gLast = 0;
new gSpotModel = 0;
new Float:gSpot[4];
new gSpotN = 0;
new gSpotVeh[8] = { INVALID_VEHICLE_ID, ... };
new gTicks = 0;

public OnFilterScriptInit()
{
    new File:f = fopen("soakcmd.txt", io_write);
    if (f) fclose(f);
    SetTimer("SoakPoll", 100, true);
    print("[soak] fs loaded");
    return 1;
}

public OnGameModeInit()
{
    // The parked cars are recreated by KeepSpot once the gmx has destroyed them (the ids are checked
    // against the model): resetting the ids here would add a second row on top of cars created
    // during the restart window.
    print("[soak] gamemode init");
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    new name[MAX_PLAYER_NAME + 1];
    GetPlayerName(playerid, name, sizeof name);
    printf("[soak] left id=%d name=%s reason=%d", playerid, name, reason);
    return 1;
}

public OnPlayerDeath(playerid, killerid, WEAPON:reason)
{
    printf("[soak] death id=%d killer=%d reason=%d", playerid, killerid, _:reason);
    return 1;
}

public OnPlayerStateChange(playerid, PLAYER_STATE:newstate, PLAYER_STATE:oldstate)
{
    printf("[soak] state id=%d %d->%d", playerid, _:oldstate, _:newstate);
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

Float:fl(const line[], n) { new b[32]; field(line, n, b); return floatstr(b); }
in(const line[], n) { new b[32]; field(line, n, b); return strval(b); }

byname(const name[])
{
    new n[MAX_PLAYER_NAME + 1];
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (!IsPlayerConnected(i)) continue;
        GetPlayerName(i, n, sizeof n);
        if (!strcmp(n, name, false)) return i;
    }
    return INVALID_PLAYER_ID;
}

Dump(seq, const who[])
{
    new local = byname(who);
    printf("[soak] %d dump begin local=%d tick=%d", seq, local, GetTickCount());
    new name[MAX_PLAYER_NAME + 1], Float:x, Float:y, Float:z, Float:vx, Float:vy, Float:vz, Float:hp, Float:ar;
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (!IsPlayerConnected(i)) continue;
        GetPlayerName(i, name, sizeof name);
        GetPlayerPos(i, x, y, z);
        GetPlayerVelocity(i, vx, vy, vz);
        GetPlayerHealth(i, hp);
        GetPlayerArmour(i, ar);
        new streamed = (local != INVALID_PLAYER_ID && i != local) ? (IsPlayerStreamedIn(i, local) ? 1 : 0) : -1;
        printf("[soak] %d p id=%d name=%s st=%d pos=%.2f,%.2f,%.2f vel=%.4f,%.4f,%.4f hp=%.1f ar=%.1f veh=%d seat=%d streamed=%d vw=%d int=%d",
            seq, i, name, _:GetPlayerState(i), x, y, z, vx, vy, vz, hp, ar, GetPlayerVehicleID(i), GetPlayerVehicleSeat(i),
            streamed, GetPlayerVirtualWorld(i), GetPlayerInterior(i));
    }
    for (new v = 1; v < MAX_VEHICLES; v++)
    {
        new model = GetVehicleModel(v);
        if (model == 0) continue;
        GetVehiclePos(v, x, y, z);
        GetVehicleVelocity(v, vx, vy, vz);
        GetVehicleHealth(v, hp);
        new streamed = (local != INVALID_PLAYER_ID) ? (IsVehicleStreamedIn(v, local) ? 1 : 0) : -1;
        printf("[soak] %d v id=%d model=%d pos=%.2f,%.2f,%.2f vel=%.4f,%.4f,%.4f hp=%.1f streamed=%d", seq, v, model, x, y, z, vx, vy, vz, hp, streamed);
    }
    printf("[soak] %d dump end tick=%d", seq, GetTickCount());
}

KeepSpot()
{
    if (gSpotModel == 0) return;
    for (new i = 0; i < gSpotN; i++)
    {
        new v = gSpotVeh[i];
        if (v != INVALID_VEHICLE_ID && GetVehicleModel(v) == gSpotModel) continue;
        new Float:a = gSpot[3] * 3.14159265 / 180.0;
        // A row along the heading's right-hand side, 5 m apart.
        new Float:off = 5.0 * float(i);
        gSpotVeh[i] = CreateVehicle(gSpotModel, gSpot[0] + off * floatcos(a, radian), gSpot[1] + off * floatsin(a, radian), gSpot[2], gSpot[3], -1, -1, 30);
        printf("[soak] spot vehicle %d slot %d", gSpotVeh[i], i);
    }
}

Run(seq, const line[])
{
    new cmd[24], a[256];
    field(line, 0, cmd);
    if (!strcmp(cmd, "dump")) { field(line, 1, a); Dump(seq, a); }
    else if (!strcmp(cmd, "kick"))
    {
        field(line, 1, a);
        new p = byname(a);
        printf("[soak] %d kick %d", seq, p);
        if (p != INVALID_PLAYER_ID) Kick(p);
    }
    else if (!strcmp(cmd, "rcon")) { field(line, 1, a); SendRconCommand(a); }
    else if (!strcmp(cmd, "vehspot"))
    {
        gSpotModel = in(line, 1);
        gSpot[0] = fl(line, 2); gSpot[1] = fl(line, 3); gSpot[2] = fl(line, 4); gSpot[3] = fl(line, 5);
        gSpotN = in(line, 6);
        if (gSpotN > sizeof gSpotVeh) gSpotN = sizeof gSpotVeh;
        KeepSpot();
    }
    else printf("[soak] %d unknown command '%s'", seq, cmd);
    printf("[soak] %d done %s", seq, cmd);
}

forward SoakPoll();
public SoakPoll()
{
    if (++gTicks % 50 == 0) KeepSpot();
    new File:f = fopen("soakcmd.txt", io_read);
    if (!f) return 1;
    new line[512];
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
