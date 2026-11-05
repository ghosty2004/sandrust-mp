// Object-map stress filterscript (CRASH-1): reproduces what a public server does when a player teleports to an
// island built from objects (e.g. "Halloween Island" at 4010, 2118, 3.5): a burst of create (44), material (84),
// move (99) and destroy (47) RPCs within about one second, then streamer-style churn as the player moves.
// Fully self-driving and deterministic (fixed LCG seed), so the original client and ours get the same input.
//   Phase 1 (global map): OBJMAP_GLOBAL global objects on the island (common map models and samp's custom
//     models 18631+ / 19xxx, particle and light models), materials and material text on some, MoveObject on
//     some right after creation, while the player is still on the mainland; then SetPlayerPos to the island.
//   Phase 2 (streamer burst): OBJMAP_STREAM player objects around the arrival point in the same tick, with
//     destroy + re-create churn of global ids.
//   Phase 3 (churn): every 100 ms for OBJMAP_CHURN_STEPS steps: destroy 25 player objects, create 25 new
//     ones (different models / materials), MoveObject on 20.
//   Phase 4 (streamer walk): OBJMAP_HOPS hops between points on the island; per hop the player objects
//     farther than 250 m from the new point are destroyed and new ones created near it (pool limit kept).
//   Phase 5: back to the mainland (everything destroyed), then to the island again (full re-create).
// Output (server log): "[objmap] phase N ...", "[objmap] alive SECS updates N" (OnPlayerUpdate count
// of the last second, so a dead client shows "updates 0"), "[objmap] done".
// Defines (pawncc sym=value): OBJMAP_START_MS (delay after the spawn, default 3000), OBJMAP_GLOBAL (400),
// OBJMAP_STREAM (480), OBJMAP_CHURN_STEPS (100), OBJMAP_HOPS (6), OBJMAP_ROUNDS (rounds of phases 3-5, 2).
#define FILTERSCRIPT
#pragma dynamic 65536
#include <open.mp>

#if !defined OBJMAP_START_MS
    #define OBJMAP_START_MS 3000
#endif
#if !defined OBJMAP_GLOBAL
    #define OBJMAP_GLOBAL 400
#endif
#if !defined OBJMAP_STREAM
    #define OBJMAP_STREAM 480
#endif
#if !defined OBJMAP_CHURN_STEPS
    #define OBJMAP_CHURN_STEPS 100
#endif
#if !defined OBJMAP_HOPS
    #define OBJMAP_HOPS 6
#endif
#if !defined OBJMAP_ROUNDS
    #define OBJMAP_ROUNDS 2
#endif

#define ISL_X 4010.0
#define ISL_Y 2118.0
#define ISL_Z 3.5
#define MAX_POBJ 600

new gPlayer = INVALID_PLAYER_ID;
new gSeed = 12345;
new gGlobal[OBJMAP_GLOBAL];
new gPobj[MAX_POBJ];
new Float:gPobjPos[MAX_POBJ][2];
new gUpdates = 0;
new gAliveSecs = 0;
new gStep = 0;
new gRound = 0;
new Float:gHere[2];

// Common SA map models, samp 0.3.x custom models (18631+ / 19xxx: floors, walls, pumpkins, coffins, lights,
// skulls, fences) and particle / light objects (fire, smoke, blood, flares) that Halloween maps use a lot.
new const gModels[] = {
    19377, 19378, 19379, 19447, 19448, 19449, 19450, 19353, 19354, 19355, 19356, 19357, 19358, 19359, 19360,
    19320, 19339, 19340, 19527, 19528, 19347, 19348, 19349, 19350, 19351, 19352, 19527, 19631, 19632, 19633,
    18631, 18632, 18633, 18634, 18635, 18636, 18637, 18638, 18639, 18640, 18641, 18642, 18643, 18644, 18645,
    18646, 18647, 18648, 18649, 18650, 18651, 18652, 18653, 18654, 18655, 18656, 18657, 18658, 18659, 18660,
    18728, 18668, 18669, 18670, 18671, 18672, 18673, 18674, 18675, 18676, 18677, 18678, 18679, 18680, 18681,
    18682, 18683, 18684, 18685, 18686, 18687, 18688, 18689, 18690, 18691, 18692, 18693, 18694, 18695, 18696,
    18697, 18698, 18699, 18700, 18701, 18702, 18703, 18704, 18705, 18706, 18707, 18708, 18709, 18710, 18711,
    18712, 18713, 18714, 18715, 18716, 18717, 18718, 18719, 18720, 18721, 18722, 18723, 18724, 18725, 18726,
    18727, 18728, 18729, 18730, 18731, 18732, 18733, 18734, 18735, 18736, 18737, 18738, 18739, 18740, 18741,
    18742, 18743, 18744, 18745, 18746, 18747, 18748, 18749, 18750, 18751, 18752, 18753, 18754, 18755, 18756,
    18757, 18758, 18759, 18760, 18761, 18762, 18763, 18764, 18765, 18766, 18767, 18768, 18769, 18770, 18771,
    19128, 19129, 19130, 19131, 19143, 19144, 19145, 19146, 19147, 19148, 19149, 19150, 19151, 19152, 19153,
    19281, 19282, 19283, 19284, 19285, 19286, 19287, 19288, 19289, 19290, 19291, 19292, 19293, 19294, 19295,
    19296, 19297, 19298, 19299, 19300, 19301, 19302, 19303, 19304, 19305, 19306, 19307, 19308, 19309, 19310,
    19311, 19312, 19313, 19314, 19315, 19316, 19317, 19318, 19319, 19321, 19322, 19323, 19324, 19325, 19326,
    3095, 3096, 3036, 3037, 8172, 8171, 8040, 8357, 898, 899, 900, 901, 902, 903, 904, 905, 906, 616, 617,
    618, 654, 655, 656, 657, 658, 659, 660, 661, 664, 669, 670, 671, 672, 673, 674, 675, 676, 677, 678, 679,
    680, 681, 682, 683, 684, 685, 686, 687, 688, 689, 690, 691, 692, 693, 694, 695, 696, 697, 698, 699, 700,
    1225, 1226, 1231, 1232, 1233, 1234, 1235, 1236, 1237, 1238, 1239, 1240, 1241, 1242, 1243, 1244, 1245,
    1290, 1294, 1297, 1298, 1408, 1409, 1410, 1411, 1412, 1413, 1414, 1415, 1416, 1417, 1418, 1419, 1420,
    3461, 3524, 3525, 3526, 3527, 3528, 3864, 3877, 3472, 1215, 1213, 2045, 2060, 2062, 2063, 2064, 2065
};

new const gTxd[][] = { "all_walls", "gta_brokentrees", "vgsehseing1", "law_beach2", "lasroads", "sw_church" };
new const gTexName[][] = { "mp_burn_wall", "CJ_bark", "ws_rooftarmac1", "sandnew_law", "Tar_1line256HV", "ws_ironfence" };
new const gTexModel[] = { 3095, 615, 9524, 6283, 5779, 13187 };

rnd(n)
{
    gSeed = (gSeed * 1103515245 + 12345) & 0x7FFFFFFF;
    return (gSeed >> 8) % n;
}

Float:frnd(Float:span)
{
    return float(rnd(20001) - 10000) / 10000.0 * span;
}

decorate(bool:player, id, i)
{
    // Materials on every 5th, material text on every 11th, both on every 55th.
    if (i % 5 == 0) {
        new t = rnd(sizeof(gTexModel));
        if (player) {
            SetPlayerObjectMaterial(gPlayer, id, 0, gTexModel[t], gTxd[t], gTexName[t], 0xFFFF8000);
        } else {
            SetObjectMaterial(id, 0, gTexModel[t], gTxd[t], gTexName[t], 0xFFFFFFFF);
        }
    }
    if (i % 11 == 0) {
        new text[32];
        format(text, sizeof(text), "{FF0000}Boo %d", i);
        if (player) {
            SetPlayerObjectMaterialText(gPlayer, id, text, 0, OBJECT_MATERIAL_SIZE_256x128, "Arial", 24, true, 0xFFFFFFFF, 0xFF000000, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
        } else {
            SetObjectMaterialText(id, text, 1, OBJECT_MATERIAL_SIZE_256x128, "Arial", 20, false, 0xFFFF8800, 0, OBJECT_MATERIAL_TEXT_ALIGN_LEFT);
        }
    }
}

createPobj(slot, Float:cx, Float:cy, Float:spread, i)
{
    new Float:x = cx + frnd(spread), Float:y = cy + frnd(spread);
    new m = gModels[rnd(sizeof(gModels))];
    new Float:dd = (i % 3 == 0) ? 0.0 : 300.0;
    gPobj[slot] = CreatePlayerObject(gPlayer, m, x, y, ISL_Z + frnd(3.0), 0.0, 0.0, float(rnd(360)), dd);
    gPobjPos[slot][0] = x;
    gPobjPos[slot][1] = y;
    if (gPobj[slot] != INVALID_OBJECT_ID) {
        decorate(true, gPobj[slot], i);
        if (i % 7 == 0) {
            MovePlayerObject(gPlayer, gPobj[slot], x + 5.0, y, ISL_Z + 2.0, 2.0 + float(rnd(5)), 0.0, 0.0, float(rnd(360)));
        }
    }
}

destroyPobj(slot)
{
    if (gPobj[slot] != INVALID_OBJECT_ID) {
        DestroyPlayerObject(gPlayer, gPobj[slot]);
        gPobj[slot] = INVALID_OBJECT_ID;
    }
}

public OnFilterScriptInit()
{
    for (new i = 0; i < MAX_POBJ; i++) gPobj[i] = INVALID_OBJECT_ID;
    for (new i = 0; i < OBJMAP_GLOBAL; i++) gGlobal[i] = INVALID_OBJECT_ID;
    SetTimer("ObjmapAlive", 1000, true);
    print("[objmap] loaded");
    return 1;
}

public OnPlayerUpdate(playerid)
{
    if (playerid == gPlayer) gUpdates++;
    return 1;
}

public OnPlayerSpawn(playerid)
{
    if (gPlayer == INVALID_PLAYER_ID && !IsPlayerNPC(playerid)) {
        gPlayer = playerid;
        printf("[objmap] player %d spawned; start in %d ms", playerid, OBJMAP_START_MS);
        SetTimer("ObjmapPhase1", OBJMAP_START_MS, false);
    }
    return 1;
}

public OnPlayerDisconnect(playerid, reason)
{
    if (playerid == gPlayer) {
        printf("[objmap] player %d disconnected reason %d", playerid, reason);
    }
    return 1;
}

forward ObjmapAlive();
public ObjmapAlive()
{
    if (gPlayer == INVALID_PLAYER_ID) return 1;
    gAliveSecs++;
    new Float:x, Float:y, Float:z;
    GetPlayerPos(gPlayer, x, y, z);
    printf("[objmap] alive %d updates %d pos %.1f %.1f %.1f", gAliveSecs, gUpdates, x, y, z);
    gUpdates = 0;
    return 1;
}

forward ObjmapPhase1();
public ObjmapPhase1()
{
    for (new i = 0; i < OBJMAP_GLOBAL; i++) {
        new Float:x = ISL_X + frnd(180.0), Float:y = ISL_Y + frnd(180.0);
        new m = gModels[rnd(sizeof(gModels))];
        gGlobal[i] = CreateObject(m, x, y, ISL_Z + frnd(4.0), 0.0, 0.0, float(rnd(360)), (i % 4 == 0) ? 0.0 : 250.0);
        decorate(false, gGlobal[i], i);
        if (i % 6 == 0) {
            MoveObject(gGlobal[i], x, y + 8.0, ISL_Z + 3.0, 1.5 + float(rnd(4)), 0.0, 0.0, float(rnd(360)));
        }
    }
    printf("[objmap] phase 1 global %d", OBJMAP_GLOBAL);
    SetTimer("ObjmapTeleport", 1000, false);
    return 1;
}

forward ObjmapTeleport();
public ObjmapTeleport()
{
    gHere[0] = ISL_X + 0.9;
    gHere[1] = ISL_Y;
    SetPlayerPos(gPlayer, gHere[0], gHere[1], ISL_Z);
    GameTextForPlayer(gPlayer, "~g~Welcome to ~r~~h~Object Island", 3500, 3);
    // Streamer burst in the same tick as the teleport.
    for (new i = 0; i < OBJMAP_STREAM; i++) createPobj(i, gHere[0], gHere[1], 120.0, i);
    // Global id churn: destroy and re-create every 10th global object at once.
    for (new i = 0; i < OBJMAP_GLOBAL; i += 10) {
        DestroyObject(gGlobal[i]);
        new Float:x = ISL_X + frnd(100.0), Float:y = ISL_Y + frnd(100.0);
        gGlobal[i] = CreateObject(gModels[rnd(sizeof(gModels))], x, y, ISL_Z, 0.0, 0.0, 0.0, 200.0);
        decorate(false, gGlobal[i], i + 1);
        MoveObject(gGlobal[i], x + 3.0, y, ISL_Z + 1.0, 3.0);
    }
    printf("[objmap] phase 2 teleport + stream %d", OBJMAP_STREAM);
    gStep = 0;
    SetTimer("ObjmapChurn", 100, false);
    return 1;
}

forward ObjmapChurn();
public ObjmapChurn()
{
    for (new k = 0; k < 25; k++) {
        new s = rnd(OBJMAP_STREAM);
        destroyPobj(s);
        createPobj(s, gHere[0], gHere[1], 120.0, gStep * 25 + k);
    }
    for (new k = 0; k < 20; k++) {
        new s = rnd(OBJMAP_STREAM);
        if (gPobj[s] != INVALID_OBJECT_ID) {
            MovePlayerObject(gPlayer, gPobj[s], gPobjPos[s][0] + frnd(5.0), gPobjPos[s][1] + frnd(5.0), ISL_Z + frnd(3.0), 4.0, frnd(180.0), frnd(180.0), frnd(180.0));
        }
    }
    gStep++;
    if (gStep < OBJMAP_CHURN_STEPS) {
        SetTimer("ObjmapChurn", 100, false);
    } else {
        printf("[objmap] phase 3 churn %d steps", gStep);
        gStep = 0;
        SetTimer("ObjmapHop", 500, false);
    }
    return 1;
}

forward ObjmapHop();
public ObjmapHop()
{
    new Float:pts[4][2] = { { 3750.0, 2118.0 }, { 4010.0, 2380.0 }, { 4270.0, 2118.0 }, { 4010.0, 1860.0 } };
    gHere[0] = pts[gStep % 4][0];
    gHere[1] = pts[gStep % 4][1];
    SetPlayerPos(gPlayer, gHere[0], gHere[1], ISL_Z + 1.0);
    new destroyed = 0, created = 0;
    for (new s = 0; s < OBJMAP_STREAM; s++) {
        new Float:dx = gPobjPos[s][0] - gHere[0], Float:dy = gPobjPos[s][1] - gHere[1];
        if (gPobj[s] == INVALID_OBJECT_ID || dx * dx + dy * dy > 250.0 * 250.0) {
            destroyPobj(s);
            destroyed++;
        }
    }
    for (new s = 0; s < OBJMAP_STREAM; s++) {
        if (gPobj[s] == INVALID_OBJECT_ID) {
            createPobj(s, gHere[0], gHere[1], 150.0, s + gStep * 1000);
            created++;
        }
    }
    printf("[objmap] hop %d to %.0f %.0f destroyed %d created %d", gStep, gHere[0], gHere[1], destroyed, created);
    gStep++;
    if (gStep < OBJMAP_HOPS) {
        SetTimer("ObjmapHop", 1500, false);
    } else {
        printf("[objmap] phase 4 hops %d", gStep);
        SetTimer("ObjmapAway", 1500, false);
    }
    return 1;
}

forward ObjmapAway();
public ObjmapAway()
{
    SetPlayerPos(gPlayer, 1700.0, -1900.0, 13.6);
    for (new s = 0; s < MAX_POBJ; s++) destroyPobj(s);
    for (new i = 0; i < OBJMAP_GLOBAL; i++) {
        DestroyObject(gGlobal[i]);
        gGlobal[i] = INVALID_OBJECT_ID;
    }
    printf("[objmap] phase 5 away, round %d", gRound);
    gRound++;
    if (gRound < OBJMAP_ROUNDS) {
        SetTimer("ObjmapPhase1", 2000, false);
    } else {
        SetTimer("ObjmapDone", 3000, false);
    }
    return 1;
}

forward ObjmapDone();
public ObjmapDone()
{
    print("[objmap] done");
    return 1;
}
