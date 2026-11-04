// M4 e2e helper filterscript: chat lines with colour codes and a game text for
// the in-game client, and logs of what the client sends (chat, commands).
// Loaded next to e2eprobe by tools/e2e/m4.sh.
//  - 2 s after a spawn: three SendClientMessage lines (plain, embedded {RRGGBB}
//    codes, a colour with alpha AA) and GameTextForPlayer style 3.
//  - OnPlayerText / OnPlayerCommandText: "[m4] text id=… text=…" / "[m4] cmd id=… cmd=…".
//  - "/m4ping": answered with a client message (server-side command round trip).
#define FILTERSCRIPT
#include <open.mp>

forward M4Greet(playerid);

public OnFilterScriptInit() { print("[m4] helper loaded"); return 1; }

public OnPlayerSpawn(playerid)
{
    SetTimerEx("M4Greet", 2000, false, "i", playerid);
    return 1;
}

public M4Greet(playerid)
{
    if (!IsPlayerConnected(playerid)) return 0;
    SendClientMessage(playerid, 0xFFFFFFFF, "M4 white line");
    SendClientMessage(playerid, 0xFF0000FF, "M4 red {00FF00}green {0000FF}blue");
    SendClientMessage(playerid, 0x33CCFFAA, "M4 alpha cyan");
    GameTextForPlayer(playerid, "~r~M4 ~w~game text", 15000, 3);
    printf("[m4] greeted id=%d", playerid);
    return 1;
}

public OnPlayerText(playerid, text[])
{
    printf("[m4] text id=%d text=%s", playerid, text);
    return 1;
}

public OnPlayerCommandText(playerid, cmdtext[])
{
    printf("[m4] cmd id=%d cmd=%s", playerid, cmdtext);
    if (!strcmp(cmdtext, "/m4ping", true))
    {
        SendClientMessage(playerid, 0xFFFF00FF, "M4 pong");
        return 1;
    }
    return 0;
}
