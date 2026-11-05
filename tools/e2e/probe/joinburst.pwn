// Join-burst filterscript (HANG-1): reproduces what a large public server sends from OnPlayerConnect, before
// the player has even reached class selection: hundreds of RemoveBuildingForPlayer (43), global and player
// textdraws with long strings shown at once (134/73), long client messages and game texts (93/73), map icons
// (56), player objects (44), a dialog with a several-KB body (61, larger than one datagram: split packet) and
// a few more large RPCs. Deterministic, so the original client and ours get the same input.
// Output (server log): "[joinburst] sent N calls to ID", "[joinburst] alive SECS updates N" (OnPlayerUpdate
// count of the last second; class selection sends no updates, so it is the client's own `[e2e] class` line
// and the control channel that prove progress), "[joinburst] dialog response ...".
// Defines (pawncc sym=value; 0 disables a part): JB_REMOVE (400), JB_TD (300 global textdraws), JB_PTD (200
// player textdraws), JB_MSG (60), JB_ICON (100), JB_POBJ (300), JB_DIALOG (1 = one 4 KB dialog), JB_VEH (1 =
// one vehicle of every model 400-611 plus extra ones far away, so InitGame's vehicle-model table is full and the
// client preloads 80 vehicle models while joining).
#define FILTERSCRIPT
#pragma dynamic 65536
#include <open.mp>

#if !defined JB_REMOVE
    #define JB_REMOVE 400
#endif
#if !defined JB_TD
    #define JB_TD 300
#endif
#if !defined JB_PTD
    #define JB_PTD 200
#endif
#if !defined JB_MSG
    #define JB_MSG 60
#endif
#if !defined JB_ICON
    #define JB_ICON 100
#endif
#if !defined JB_POBJ
    #define JB_POBJ 300
#endif
#if !defined JB_VEH
    #define JB_VEH 1
#endif
#if !defined JB_DIALOG
    #define JB_DIALOG 1
#endif

new gSeed = 4242;
new gUpdates[MAX_PLAYERS];
new gAlive = 0;
#if JB_TD > 0
new Text:gTd[JB_TD];
#endif

rnd(n)
{
    gSeed = (gSeed * 1103515245 + 12345) & 0x7FFFFFFF;
    return (gSeed >> 8) % n;
}

Float:frnd(Float:span)
{
    return float(rnd(20001) - 10000) / 10000.0 * span;
}

// Models that the large RP maps remove most (Los Santos / San Fierro buildings, lamps, trees, fences).
new const gRemoveModels[] = {
    1226, 1290, 1294, 1297, 1283, 1284, 1315, 1350, 1352, 700, 620, 615, 673, 647, 3459, 3460, 1468, 1408,
    4019, 4025, 4215, 4216, 4217, 4218, 5024, 5044, 5048, 5409, 5681, 5772, 5856, 6046, 6098, 6203, 10671,
    11372, 11451, 11452, 13028, 13190, 17513, 17526, 17530, 17535, 17545, 17616, 17946, 17949, 18020, 18025
};

public OnFilterScriptInit()
{
    print("[joinburst] loaded");
#if JB_TD > 0
    new text[200];
    for (new i = 0; i < JB_TD; i++) {
        format(text, sizeof(text), "~r~JOIN~w~BURST_%03d_~y~LOREM_IPSUM_DOLOR_SIT_AMET_CONSECTETUR_ADIPISCING_ELIT_SED_DO_EIUSMOD_TEMPOR_INCIDIDUNT_UT_LABORE_ET_DOLORE_MAGNA_ALIQUA_~g~%d", i, i * 7);
        gTd[i] = TextDrawCreate(10.0 + float(i % 20) * 30.0, 100.0 + float(i / 20) * 20.0, text);
        TextDrawFont(gTd[i], TEXT_DRAW_FONT:(i % 4));
        TextDrawLetterSize(gTd[i], 0.2, 0.8);
        TextDrawUseBox(gTd[i], (i % 3) == 0);
        TextDrawBoxColour(gTd[i], 0x00000066);
        TextDrawSetOutline(gTd[i], 1);
        TextDrawColour(gTd[i], 0xFFFFFFFF);
    }
#endif
#if JB_VEH > 0
    // Parked far out at sea, never streamed to the player during the test.
    new nveh = 0;
    for (new m = 400; m <= 611; m++) {
        for (new k = 0; k < 1 + (m % 5); k++) {
            if (CreateVehicle(m, -2900.0 + float(nveh % 40) * 8.0, 2900.0 - float(nveh / 40) * 8.0, 2.0, 0.0, -1, -1, -1) != INVALID_VEHICLE_ID) {
                nveh++;
            }
        }
    }
    printf("[joinburst] %d vehicles", nveh);
#endif
    SetTimer("jb_alive", 1000, true);
    return 1;
}

forward jb_alive();
public jb_alive()
{
    gAlive++;
    for (new i = 0; i < MAX_PLAYERS; i++) {
        if (IsPlayerConnected(i)) {
            printf("[joinburst] alive %d updates %d id=%d", gAlive, gUpdates[i], i);
            gUpdates[i] = 0;
        }
    }
}

public OnPlayerUpdate(playerid)
{
    gUpdates[playerid]++;
    return 1;
}

public OnPlayerConnect(playerid)
{
    if (IsPlayerNPC(playerid)) {
        return 1;
    }
    gSeed = 4242;
    new calls = 0;
#if JB_REMOVE > 0
    for (new i = 0; i < JB_REMOVE; i++) {
        new m = gRemoveModels[rnd(sizeof(gRemoveModels))];
        if (i % 97 == 0) {
            m = -1;
        }
        new Float:r = (i % 10 == 0) ? 50.0 : 0.25;
        new Float:x = 1500.0 + frnd(1500.0), Float:y = -1500.0 + frnd(1500.0);
        // The first 169 lie on a 500-unit grid over the whole map, like maps spread over every city: every map
        // occluder is within 500 units of one of them.
        if (i < 169) {
            x = -3000.0 + float(i % 13) * 500.0;
            y = -3000.0 + float(i / 13) * 500.0;
        }
        RemoveBuildingForPlayer(playerid, m, x, y, 10.0 + frnd(10.0), r);
        calls++;
    }
#endif
    new text[160];
#if JB_MSG > 0
    for (new i = 0; i < JB_MSG; i++) {
        format(text, sizeof(text), "{FFFF00}[JoinBurst] {FFFFFF}message %d: the quick brown fox jumps over the lazy dog again and again, {00FF00}%d{FFFFFF} times.", i, i * 13);
        SendClientMessage(playerid, 0xFFFFFFFF, text);
        calls++;
    }
#endif
    GameTextForPlayer(playerid, "~w~Welcome to ~r~JoinBurst~n~~y~a very long game text with several lines~n~~g~and colours~n~~b~to check the parser", 5000, 3);
    calls++;
#if JB_TD > 0
    for (new i = 0; i < JB_TD; i++) {
        TextDrawShowForPlayer(playerid, gTd[i]);
        calls++;
    }
#endif
#if JB_PTD > 0
    for (new i = 0; i < JB_PTD; i++) {
        format(text, sizeof(text), "~b~PLAYER_TD_%03d~w~_SOME_HUD_ELEMENT_WITH_A_LONG_LABEL_AND_A_VALUE_OF_%d_~r~$", i, i * 1000);
        new PlayerText:t = CreatePlayerTextDraw(playerid, 320.0 + float(i % 10) * 25.0, 20.0 + float(i / 10) * 15.0, text);
        PlayerTextDrawLetterSize(playerid, t, 0.25, 1.0);
        PlayerTextDrawFont(playerid, t, TEXT_DRAW_FONT:((i % 3) + 1));
        PlayerTextDrawShow(playerid, t);
        calls += 2;
    }
#endif
#if JB_ICON > 0
    for (new i = 0; i < JB_ICON; i++) {
        SetPlayerMapIcon(playerid, i, frnd(3000.0), frnd(3000.0), 10.0, 5 + (i % 50), 0, MAPICON_LOCAL);
        calls++;
    }
#endif
#if JB_POBJ > 0
    for (new i = 0; i < JB_POBJ; i++) {
        CreatePlayerObject(playerid, 1225 + (i % 20), 1500.0 + frnd(500.0), -1600.0 + frnd(500.0), 13.0, 0.0, 0.0, float(i % 360), 300.0);
        calls++;
    }
#endif
#if JB_DIALOG > 0
    new body[4096];
    for (new i = 0; i < 40; i++) {
        format(text, sizeof(text), "{FFFFFF}Rule %02d: {AAAAAA}be nice to the other players and do not use cheats or bugs (%d)\n", i, i);
        strcat(body, text);
    }
    ShowPlayerDialog(playerid, 77, DIALOG_STYLE_MSGBOX, "{FF0000}JoinBurst rules", body, "Accept", "Quit");
    calls++;
    printf("[joinburst] dialog body %d bytes", strlen(body));
#endif
    printf("[joinburst] sent %d calls to %d", calls, playerid);
    return 1;
}

public OnDialogResponse(playerid, dialogid, response, listitem, inputtext[])
{
    printf("[joinburst] dialog response id=%d dialog=%d response=%d", playerid, dialogid, response);
    return 0;
}
