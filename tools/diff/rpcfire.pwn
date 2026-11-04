// DIFF-2 RPC coverage filterscript (diff lane only).
//
// Driven by tools/diff/drive.py over RCON (OnRconCommand):
//   diff list          prints every step:  [diff] step <idx> <rpcs> <name> <settle_ms> <action>
//   diff fire <name> ; runs ONE step against the test player, prints
//                      [diff] fire <rpcid> <name> step=<idx>   (or [diff] error ...)
// <rpcs> is the comma-separated list of S->C RPC ids the step is expected to
// make the server send (first = primary); "0" for the idle baseline steps.
// <action> "fs" = this script calls the native; "key:<xdotool key>" = the
// harness presses that key (RPCs answered by the server to a client request);
// "flow" = covered by the join/spawn flow before the steps.
// Every argument is a fixed constant: both clients must get identical RPCs.
#define FILTERSCRIPT
#include <open.mp>

#define BX 1958.3783
#define BY 1343.1572
#define BZ 15.3746
#define BA 270.0

enum E_STEP { E_RPCS[24], E_NAME[32], E_SETTLE, E_ACTION[16] }

// Order matters: later steps rely on entities created by earlier ones, and
// the destructive steps (death, class selection, gmx) come last.
new const g_Steps[][E_STEP] = {
    {"139,60,128", "flow_join", 0, "flow"},
    {"129,68", "flow_spawn", 0, "flow"},
    {"0", "idle_a", 1500, "fs"},
    {"0", "idle_b", 1500, "fs"},
    // --- local player ---------------------------------------------------
    {"14", "health_50", 1200, "fs"},
    {"66", "armour_75", 1200, "fs"},
    {"18", "money_give_12345", 1500, "fs"},
    {"20", "money_reset", 1500, "fs"},
    {"18", "money_give_500", 1500, "fs"},
    {"22", "weapon_give_deagle", 1500, "fs"},
    {"22", "weapon_give_m4", 1500, "fs"},
    {"67", "weapon_arm_deagle", 1500, "fs"},
    {"145", "weapon_ammo_m4", 1200, "fs"},
    {"21", "weapons_reset", 1500, "fs"},
    {"19", "facing_90", 1200, "fs"},
    {"12", "pos_offset", 1500, "fs"},
    {"13", "pos_findz", 1500, "fs"},
    {"153", "skin_101", 2000, "fs"},
    {"89", "fightstyle_kungfu", 1200, "fs"},
    {"34", "skill_pistol_500", 1200, "fs"},
    {"35", "drunk_5000", 2000, "fs"},
    {"35", "drunk_0", 1500, "fs"},
    {"133", "wanted_3", 1500, "fs"},
    {"133", "wanted_0", 1500, "fs"},
    {"69", "team_3", 1200, "fs"},
    {"72", "colour_red", 1200, "fs"},
    {"11", "name_renamed", 1200, "fs"},
    {"11", "name_restore", 1200, "fs"},
    {"156", "interior_1", 2000, "fs"},
    {"156", "interior_0", 2000, "fs"},
    {"48", "vworld_7", 1200, "fs"},
    {"48", "vworld_0", 1500, "fs"},
    {"29", "time_22_30", 1500, "fs"},
    {"30", "clock_on", 1500, "fs"},
    {"30", "clock_off", 1500, "fs"},
    {"29", "time_12_00", 1500, "fs"},
    {"94", "worldtime_15", 1500, "fs"},
    {"152", "weather_8", 2000, "fs"},
    {"152", "weather_10", 2000, "fs"},
    {"146", "gravity_low", 1200, "fs"},
    {"146", "gravity_default", 1200, "fs"},
    {"17", "bounds_small", 1200, "fs"},
    {"17", "bounds_reset", 1200, "fs"},
    {"104", "stuntbonus_off", 1200, "fs"},
    {"111", "widescreen_on", 1500, "fs"},
    {"111", "widescreen_off", 1500, "fs"},
    {"15", "controllable_off", 1200, "fs"},
    {"15", "controllable_on", 1200, "fs"},
    {"16", "sound_1057", 1200, "fs"},
    {"112", "crime_report", 2500, "fs"},
    {"33", "shopname_pizza", 1200, "fs"},
    {"33", "shopname_clear", 1200, "fs"},
    {"90", "velocity_up", 2000, "fs"},
    {"86", "anim_drunk_walk", 2000, "fs"},
    {"87", "anim_clear", 1500, "fs"},
    {"88", "specialaction_phone", 2500, "fs"},
    {"88", "specialaction_phone_stop", 2500, "fs"},
    {"113", "attached_briefcase", 2000, "fs"},
    {"116", "attached_edit", 1500, "fs"},
    {"28", "attached_edit_cancel", 1500, "fs"},
    {"113", "attached_remove", 1500, "fs"},
    {"12", "pos_base", 2000, "fs"},
    // --- UI ---------------------------------------------------------------
    {"93", "client_message", 1200, "fs"},
    {"101", "player_message", 1200, "fs"},
    {"73", "gametext_style3", 1500, "fs"},
    {"73", "gametext_style6", 1500, "fs"},
    {"55", "death_message", 1200, "fs"},
    {"61", "dialog_msgbox", 1500, "fs"},
    {"61", "dialog_input", 1500, "fs"},
    {"61", "dialog_list", 1500, "fs"},
    {"61", "dialog_password", 1500, "fs"},
    {"61", "dialog_tablist", 1500, "fs"},
    {"61", "dialog_tablist_headers", 1500, "fs"},
    {"61", "dialog_hide", 1500, "fs"},
    {"134", "ptextdraw_show", 1500, "fs"},
    {"105", "ptextdraw_setstring", 1500, "fs"},
    {"83", "textdraw_select", 1500, "fs"},
    {"83", "textdraw_select_cancel", 1500, "fs"},
    {"135", "ptextdraw_hide", 1500, "fs"},
    {"36", "label_create", 1500, "fs"},
    {"58", "label_delete", 1200, "fs"},
    {"108", "gangzone_show", 1500, "fs"},
    {"121", "gangzone_flash", 1500, "fs"},
    {"85", "gangzone_stopflash", 1500, "fs"},
    {"120", "gangzone_hide", 1500, "fs"},
    {"56", "mapicon_set", 1500, "fs"},
    {"144", "mapicon_remove", 1200, "fs"},
    {"107", "checkpoint_set", 1500, "fs"},
    {"37", "checkpoint_disable", 1200, "fs"},
    {"38", "racecp_set", 1500, "fs"},
    {"39", "racecp_disable", 1200, "fs"},
    {"76,77", "menu_show", 1500, "fs"},
    {"78", "menu_hide", 1500, "fs"},
    {"41", "audiostream_play", 2000, "fs"},
    {"42", "audiostream_stop", 1200, "fs"},
    {"103", "client_check", 1500, "fs"},
    {"79", "explosion", 2500, "fs"},
    {"170", "camtarget_on", 1200, "fs"},
    {"170", "camtarget_off", 1200, "fs"},
    // --- camera -----------------------------------------------------------
    {"157", "camera_pos", 1500, "fs"},
    {"158", "camera_lookat", 1500, "fs"},
    {"82", "camera_interp_pos", 2000, "fs"},
    {"82", "camera_interp_lookat", 2000, "fs"},
    {"162", "camera_behind", 1500, "fs"},
    // --- objects ------------------------------------------------------------
    {"44", "object_create", 2000, "fs"},
    {"45", "object_setpos", 1500, "fs"},
    {"46", "object_setrot", 1500, "fs"},
    {"99", "object_move", 1500, "fs"},
    {"122", "object_stop", 1200, "fs"},
    {"84", "object_material", 1500, "fs"},
    {"84", "object_material_text", 1500, "fs"},
    {"117", "object_edit", 1500, "fs"},
    {"28", "object_edit_cancel", 1500, "fs"},
    {"27", "object_select", 1500, "fs"},
    {"28", "object_select_cancel", 1500, "fs"},
    {"81", "camera_attach_object", 1500, "fs"},
    {"162", "camera_behind_2", 1500, "fs"},
    {"75", "object_attach_player", 1500, "fs"},
    {"47", "object_destroy", 1500, "fs"},
    // --- pickups ----------------------------------------------------------------
    {"95", "pickup_create", 2000, "fs"},
    {"63", "pickup_destroy", 1500, "fs"},
    // --- actors -------------------------------------------------------------
    {"171", "actor_create", 2500, "fs"},
    {"173", "actor_anim", 2000, "fs"},
    {"174", "actor_anim_clear", 1500, "fs"},
    {"175", "actor_facing", 1500, "fs"},
    {"176", "actor_pos", 1500, "fs"},
    {"178", "actor_health", 1200, "fs"},
    {"172", "actor_destroy", 2500, "fs"},
    // --- vehicles -------------------------------------------------------------
    {"164", "vehicle_create", 2500, "fs"},
    {"24", "vehicle_params_locked", 1500, "fs"},
    {"24", "vehicle_params_unlocked", 1500, "fs"},
    {"123", "vehicle_plate", 1500, "fs"},
    {"147", "vehicle_health", 1500, "fs"},
    {"106", "vehicle_damage", 1500, "fs"},
    {"96", "vehicle_colour", 1500, "fs"},
    {"96", "vehicle_component_add", 1500, "fs"},
    {"57", "vehicle_component_remove", 1500, "fs"},
    {"160", "vehicle_zangle", 1500, "fs"},
    {"159", "vehicle_pos", 1500, "fs"},
    {"65", "vehicle_interior_1", 1500, "fs"},
    {"65", "vehicle_interior_0", 1500, "fs"},
    {"70", "vehicle_put_player", 3000, "fs"},
    {"91", "vehicle_velocity", 2000, "fs"},
    {"167", "vehicle_collisions_off", 1200, "fs"},
    {"167", "vehicle_collisions_on", 1200, "fs"},
    {"71", "vehicle_remove_player", 3500, "fs"},
    {"164", "vehicle_create_truck_trailer", 2500, "fs"},
    {"148", "trailer_attach", 2000, "fs"},
    {"149", "trailer_detach", 2000, "fs"},
    {"165", "vehicle_destroy_trailer", 2500, "fs"},
    {"165", "vehicle_destroy_truck", 2500, "fs"},
    {"12", "pos_base_2", 2000, "fs"},
    // --- remote player (open.mp NPC) -------------------------------------------
    {"137", "npc_create", 1500, "fs"},
    {"32", "npc_spawn", 3000, "fs"},
    {"59", "npc_chat_bubble", 1500, "fs"},
    {"80", "npc_nametag_hide", 1500, "fs"},
    {"80", "npc_nametag_show", 1500, "fs"},
    {"72", "npc_colour_green", 1200, "fs"},
    {"26", "npc_put_in_vehicle", 2500, "fs"},
    {"154", "npc_exit_vehicle", 3000, "fs"},
    {"124", "spectate_on", 2000, "fs"},
    {"126", "spectate_npc", 2000, "fs"},
    {"127", "spectate_vehicle", 2000, "fs"},
    {"124", "spectate_off", 3500, "fs"},
    {"166", "npc_kill", 3000, "fs"},
    {"163", "npc_stream_out", 3000, "fs"},
    {"138", "npc_destroy", 1500, "fs"},
    {"165", "vehicle_destroy", 2500, "fs"},
    // --- spawn / class selection / session (destructive, last) -------------
    {"129", "spawn_immediately", 3500, "fs"},
    {"43", "remove_buildings", 2500, "fs"},
    {"68", "spawn_info", 1200, "fs"},
    {"74", "force_class_selection", 1200, "fs"},
    {"14,128", "kill_to_class_selection", 12000, "fs"},
    {"129", "class_spawn_key", 4000, "key:shift"},
    {"155", "scoreboard_key", 2500, "key:Tab"},
    {"40,139", "gamemode_restart", 20000, "fs"}
};

new g_Player = INVALID_PLAYER_ID;
new g_Name[MAX_PLAYER_NAME + 1];
new g_Obj = INVALID_OBJECT_ID;
new PlayerText:g_Td = PlayerText:INVALID_TEXT_DRAW;
new PlayerText3D:g_Label = PlayerText3D:INVALID_3DTEXT_ID;
new g_Zone = -1;
new Menu:g_Menu = Menu:INVALID_MENU;
new g_Pickup = -1;
new g_Actor = INVALID_ACTOR_ID;
new g_Veh = INVALID_VEHICLE_ID;
new g_Truck = INVALID_VEHICLE_ID;
new g_Trailer = INVALID_VEHICLE_ID;
new g_Npc = INVALID_PLAYER_ID;

public OnFilterScriptInit()
{
    printf("[diff] rpcfire loaded, %d steps", sizeof g_Steps);
    return 1;
}

// The test player: the first connected human (NPCs excluded).
FindPlayer()
{
    for (new i = 0; i < MAX_PLAYERS; i++)
    {
        if (IsPlayerConnected(i) && !IsPlayerNPC(i)) return i;
    }
    return INVALID_PLAYER_ID;
}

IsNameChar(c)
{
    return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_';
}

FindStep(const name[])
{
    for (new i = 0; i < sizeof g_Steps; i++)
    {
        if (!strcmp(g_Steps[i][E_NAME], name)) return i;
    }
    return -1;
}

public OnRconCommand(cmd[])
{
    if (strcmp(cmd, "diff ", false, 5)) return 0;
    if (!strcmp(cmd[5], "list", false, 4))
    {
        for (new i = 0; i < sizeof g_Steps; i++)
        {
            printf("[diff] step %d %s %s %d %s", i, g_Steps[i][E_RPCS], g_Steps[i][E_NAME],
                g_Steps[i][E_SETTLE], g_Steps[i][E_ACTION]);
        }
        printf("[diff] list end %d", sizeof g_Steps);
        return 1;
    }
    if (!strcmp(cmd[5], "fire ", false, 5))
    {
        // RCON text arriving through the query port is not NUL-terminated at its
        // length (stale bytes of earlier commands follow), so drive.py sends
        // "diff fire <name> ;" and only the leading [a-z0-9_] run is taken.
        new name[32], n = 0;
        while (n < sizeof name - 1 && IsNameChar(cmd[10 + n]))
        {
            name[n] = cmd[10 + n];
            n++;
        }
        name[n] = EOS;
        new idx = FindStep(name);
        if (idx < 0)
        {
            printf("[diff] error unknown step %s", name);
            return 1;
        }
        g_Player = FindPlayer();
        if (g_Player == INVALID_PLAYER_ID)
        {
            printf("[diff] error no player for step=%d", idx);
            return 1;
        }
        new primary = strval(g_Steps[idx][E_RPCS]);
        if (!Fire(g_Player, name))
        {
            printf("[diff] error step %s failed step=%d", name, idx);
            return 1;
        }
        printf("[diff] fire %d %s step=%d", primary, name, idx);
        return 1;
    }
    print("[diff] error bad command");
    return 1;
}

// Runs one step. Returns false if a precondition is missing.
Fire(p, const s[])
{
    if (!strcmp(s, "idle_a") || !strcmp(s, "idle_b")) return true;
    if (!strcmp(s, "flow_join") || !strcmp(s, "flow_spawn")) return true;
    if (!strcmp(s, "class_spawn_key") || !strcmp(s, "scoreboard_key")) return true;

    // local player
    if (!strcmp(s, "health_50")) return SetPlayerHealth(p, 50.0);
    if (!strcmp(s, "armour_75")) return SetPlayerArmour(p, 75.0);
    if (!strcmp(s, "money_give_12345")) return GivePlayerMoney(p, 12345);
    if (!strcmp(s, "money_reset")) return ResetPlayerMoney(p);
    if (!strcmp(s, "money_give_500")) return GivePlayerMoney(p, 500);
    if (!strcmp(s, "weapon_give_deagle")) return GivePlayerWeapon(p, WEAPON_DEAGLE, 50);
    if (!strcmp(s, "weapon_give_m4")) return GivePlayerWeapon(p, WEAPON_M4, 200);
    if (!strcmp(s, "weapon_arm_deagle")) return SetPlayerArmedWeapon(p, WEAPON_DEAGLE);
    if (!strcmp(s, "weapon_ammo_m4")) return SetPlayerAmmo(p, WEAPON_M4, 77);
    if (!strcmp(s, "weapons_reset")) return ResetPlayerWeapons(p);
    if (!strcmp(s, "facing_90")) return SetPlayerFacingAngle(p, 90.0);
    if (!strcmp(s, "pos_offset")) return SetPlayerPos(p, BX + 3.0, BY + 1.0, BZ);
    if (!strcmp(s, "pos_findz")) return SetPlayerPosFindZ(p, BX, BY, BZ + 40.0);
    if (!strcmp(s, "skin_101")) return SetPlayerSkin(p, 101);
    if (!strcmp(s, "fightstyle_kungfu")) return SetPlayerFightingStyle(p, FIGHT_STYLE_KUNGFU);
    if (!strcmp(s, "skill_pistol_500")) return SetPlayerSkillLevel(p, WEAPONSKILL_PISTOL, 500);
    if (!strcmp(s, "drunk_5000")) return SetPlayerDrunkLevel(p, 5000);
    if (!strcmp(s, "drunk_0")) return SetPlayerDrunkLevel(p, 0);
    if (!strcmp(s, "wanted_3")) return SetPlayerWantedLevel(p, 3);
    if (!strcmp(s, "wanted_0")) return SetPlayerWantedLevel(p, 0);
    if (!strcmp(s, "team_3")) return SetPlayerTeam(p, 3);
    if (!strcmp(s, "colour_red")) return SetPlayerColor(p, 0xFF0000FF);
    if (!strcmp(s, "name_renamed"))
    {
        GetPlayerName(p, g_Name, sizeof g_Name);
        return SetPlayerName(p, "DiffRenamed") == 1;
    }
    if (!strcmp(s, "name_restore")) return SetPlayerName(p, g_Name) == 1;
    if (!strcmp(s, "interior_1")) return SetPlayerInterior(p, 1);
    if (!strcmp(s, "interior_0")) return SetPlayerInterior(p, 0);
    if (!strcmp(s, "vworld_7")) return SetPlayerVirtualWorld(p, 7);
    if (!strcmp(s, "vworld_0")) return SetPlayerVirtualWorld(p, 0);
    if (!strcmp(s, "time_22_30")) return SetPlayerTime(p, 22, 30);
    if (!strcmp(s, "clock_on")) return TogglePlayerClock(p, true);
    if (!strcmp(s, "clock_off")) return TogglePlayerClock(p, false);
    if (!strcmp(s, "time_12_00")) return SetPlayerTime(p, 12, 0);
    if (!strcmp(s, "worldtime_15")) { SetWorldTime(15); return true; }
    if (!strcmp(s, "weather_8")) return SetPlayerWeather(p, 8);
    if (!strcmp(s, "weather_10")) return SetPlayerWeather(p, 10);
    if (!strcmp(s, "gravity_low")) return SetPlayerGravity(p, 0.004);
    if (!strcmp(s, "gravity_default")) return SetPlayerGravity(p, 0.008);
    if (!strcmp(s, "bounds_small")) return SetPlayerWorldBounds(p, BX + 20.0, BX - 20.0, BY + 20.0, BY - 20.0);
    if (!strcmp(s, "bounds_reset")) return SetPlayerWorldBounds(p, 20000.0, -20000.0, 20000.0, -20000.0);
    if (!strcmp(s, "stuntbonus_off")) return EnableStuntBonusForPlayer(p, false);
    if (!strcmp(s, "widescreen_on")) return TogglePlayerWidescreen(p, true);
    if (!strcmp(s, "widescreen_off")) return TogglePlayerWidescreen(p, false);
    if (!strcmp(s, "controllable_off")) return TogglePlayerControllable(p, false);
    if (!strcmp(s, "controllable_on")) return TogglePlayerControllable(p, true);
    if (!strcmp(s, "sound_1057")) return PlayerPlaySound(p, 1057, 0.0, 0.0, 0.0);
    if (!strcmp(s, "crime_report")) return PlayCrimeReportForPlayer(p, p, 16);
    if (!strcmp(s, "shopname_pizza")) return SetPlayerShopName(p, "FDPIZA");
    if (!strcmp(s, "shopname_clear")) return SetPlayerShopName(p, "");
    if (!strcmp(s, "velocity_up")) return SetPlayerVelocity(p, 0.0, 0.0, 0.4);
    if (!strcmp(s, "anim_drunk_walk")) return ApplyAnimation(p, "PED", "WALK_DRUNK", 4.1, true, true, true, false, 0, SYNC_NONE);
    if (!strcmp(s, "anim_clear")) return ClearAnimations(p, SYNC_NONE);
    if (!strcmp(s, "specialaction_phone")) return SetPlayerSpecialAction(p, SPECIAL_ACTION_USECELLPHONE);
    if (!strcmp(s, "specialaction_phone_stop")) return SetPlayerSpecialAction(p, SPECIAL_ACTION_STOPUSECELLPHONE);
    if (!strcmp(s, "attached_briefcase")) return SetPlayerAttachedObject(p, 0, 1210, 6, 0.3, 0.1, 0.0, 0.0, -90.0, 0.0);
    if (!strcmp(s, "attached_edit")) return EditAttachedObject(p, 0);
    if (!strcmp(s, "attached_edit_cancel") || !strcmp(s, "object_edit_cancel") || !strcmp(s, "object_select_cancel"))
    {
        CancelEdit(p);
        return true;
    }
    if (!strcmp(s, "attached_remove")) return RemovePlayerAttachedObject(p, 0);
    if (!strcmp(s, "pos_base") || !strcmp(s, "pos_base_2"))
    {
        SetPlayerPos(p, BX, BY, BZ);
        SetPlayerFacingAngle(p, BA);
        return true;
    }

    // UI
    if (!strcmp(s, "client_message")) return SendClientMessage(p, 0xFFAA00FF, "diff: client message");
    if (!strcmp(s, "player_message")) return SendPlayerMessageToPlayer(p, p, "diff: player message");
    if (!strcmp(s, "gametext_style3")) return GameTextForPlayer(p, "~r~diff ~w~game text", 4000, 3);
    if (!strcmp(s, "gametext_style6")) return GameTextForPlayer(p, "~g~diff style six", 4000, 6);
    if (!strcmp(s, "death_message")) return SendDeathMessageToPlayer(p, INVALID_PLAYER_ID, p, 54);
    if (!strcmp(s, "dialog_msgbox")) return ShowPlayerDialog(p, 1, DIALOG_STYLE_MSGBOX, "Diff msgbox", "Line one\nLine two", "OK", "Cancel");
    if (!strcmp(s, "dialog_input")) return ShowPlayerDialog(p, 2, DIALOG_STYLE_INPUT, "Diff input", "Type something:", "OK", "Cancel");
    if (!strcmp(s, "dialog_list")) return ShowPlayerDialog(p, 3, DIALOG_STYLE_LIST, "Diff list", "Alpha\nBeta\nGamma", "Pick", "Cancel");
    if (!strcmp(s, "dialog_password")) return ShowPlayerDialog(p, 4, DIALOG_STYLE_PASSWORD, "Diff password", "Password:", "OK", "Cancel");
    if (!strcmp(s, "dialog_tablist")) return ShowPlayerDialog(p, 5, DIALOG_STYLE_TABLIST, "Diff tablist", "A\t1\t$10\nB\t2\t$20", "Pick", "Cancel");
    if (!strcmp(s, "dialog_tablist_headers")) return ShowPlayerDialog(p, 6, DIALOG_STYLE_TABLIST_HEADERS, "Diff headers", "Name\tCount\tPrice\nA\t1\t$10\nB\t2\t$20", "Pick", "Cancel");
    if (!strcmp(s, "dialog_hide")) return ShowPlayerDialog(p, -1, DIALOG_STYLE_MSGBOX, "", "", "", "");
    if (!strcmp(s, "ptextdraw_show"))
    {
        g_Td = CreatePlayerTextDraw(p, 320.0, 200.0, "DIFF TEXTDRAW");
        PlayerTextDrawAlignment(p, g_Td, TEXT_DRAW_ALIGN_CENTRE);
        PlayerTextDrawLetterSize(p, g_Td, 0.6, 2.0);
        PlayerTextDrawTextSize(p, g_Td, 40.0, 200.0);
        PlayerTextDrawSetSelectable(p, g_Td, true);
        return PlayerTextDrawShow(p, g_Td);
    }
    if (!strcmp(s, "ptextdraw_setstring")) return PlayerTextDrawSetString(p, g_Td, "DIFF CHANGED");
    if (!strcmp(s, "textdraw_select")) return SelectTextDraw(p, 0xFF0000FF);
    if (!strcmp(s, "textdraw_select_cancel")) return CancelSelectTextDraw(p);
    if (!strcmp(s, "ptextdraw_hide"))
    {
        new bool:ok = PlayerTextDrawHide(p, g_Td);
        PlayerTextDrawDestroy(p, g_Td);
        return ok;
    }
    if (!strcmp(s, "label_create"))
    {
        g_Label = CreatePlayer3DTextLabel(p, "diff label", 0xFFFF00FF, BX + 4.0, BY, BZ + 1.0, 40.0);
        return g_Label != PlayerText3D:INVALID_3DTEXT_ID;
    }
    if (!strcmp(s, "label_delete")) return DeletePlayer3DTextLabel(p, g_Label);
    if (!strcmp(s, "gangzone_show"))
    {
        g_Zone = GangZoneCreate(BX - 60.0, BY - 60.0, BX + 60.0, BY + 60.0);
        return GangZoneShowForPlayer(p, g_Zone, 0xFF000080);
    }
    if (!strcmp(s, "gangzone_flash")) return GangZoneFlashForPlayer(p, g_Zone, 0x0000FF80);
    if (!strcmp(s, "gangzone_stopflash")) return GangZoneStopFlashForPlayer(p, g_Zone);
    if (!strcmp(s, "gangzone_hide"))
    {
        new bool:ok = GangZoneHideForPlayer(p, g_Zone);
        GangZoneDestroy(g_Zone);
        return ok;
    }
    if (!strcmp(s, "mapicon_set")) return SetPlayerMapIcon(p, 0, BX + 40.0, BY, BZ, 52, 0, MAPICON_GLOBAL);
    if (!strcmp(s, "mapicon_remove")) return RemovePlayerMapIcon(p, 0);
    if (!strcmp(s, "checkpoint_set")) return SetPlayerCheckpoint(p, BX + 8.0, BY, BZ, 3.0);
    if (!strcmp(s, "checkpoint_disable")) return DisablePlayerCheckpoint(p);
    if (!strcmp(s, "racecp_set")) return SetPlayerRaceCheckpoint(p, CP_TYPE_GROUND_NORMAL, BX + 10.0, BY, BZ, BX + 30.0, BY, BZ, 5.0);
    if (!strcmp(s, "racecp_disable")) return DisablePlayerRaceCheckpoint(p);
    if (!strcmp(s, "menu_show"))
    {
        g_Menu = CreateMenu("Diff menu", 1, 200.0, 150.0, 150.0);
        AddMenuItem(g_Menu, 0, "First");
        AddMenuItem(g_Menu, 0, "Second");
        return ShowMenuForPlayer(g_Menu, p);
    }
    if (!strcmp(s, "menu_hide"))
    {
        new bool:ok = HideMenuForPlayer(g_Menu, p);
        DestroyMenu(g_Menu);
        return ok;
    }
    // Port 9 (discard) on loopback: the stream fails fast and never plays.
    if (!strcmp(s, "audiostream_play")) return PlayAudioStreamForPlayer(p, "http://127.0.0.1:9/diff.mp3");
    if (!strcmp(s, "audiostream_stop")) return StopAudioStreamForPlayer(p);
    if (!strcmp(s, "client_check")) return SendClientCheck(p, 0x48, 0, 0, 2);
    if (!strcmp(s, "explosion")) return CreateExplosionForPlayer(p, BX + 25.0, BY, BZ, 12, 2.0);
    if (!strcmp(s, "camtarget_on")) return EnablePlayerCameraTarget(p, true);
    if (!strcmp(s, "camtarget_off")) return EnablePlayerCameraTarget(p, false);

    // camera
    if (!strcmp(s, "camera_pos")) return SetPlayerCameraPos(p, BX + 12.0, BY + 12.0, BZ + 6.0);
    if (!strcmp(s, "camera_lookat")) return SetPlayerCameraLookAt(p, BX, BY, BZ, CAMERA_CUT);
    if (!strcmp(s, "camera_interp_pos")) return InterpolateCameraPos(p, BX + 12.0, BY + 12.0, BZ + 6.0, BX - 12.0, BY + 12.0, BZ + 6.0, 1000, CAMERA_MOVE);
    if (!strcmp(s, "camera_interp_lookat")) return InterpolateCameraLookAt(p, BX, BY, BZ, BX + 5.0, BY, BZ, 1000, CAMERA_MOVE);
    if (!strcmp(s, "camera_behind") || !strcmp(s, "camera_behind_2")) return SetCameraBehindPlayer(p);

    // objects (per-player object 3594: burnt car shell, stock GTA model)
    if (!strcmp(s, "object_create"))
    {
        g_Obj = CreatePlayerObject(p, 3594, BX + 8.0, BY, BZ, 0.0, 0.0, 0.0, 200.0);
        return g_Obj != INVALID_OBJECT_ID;
    }
    if (!strcmp(s, "object_setpos")) return SetPlayerObjectPos(p, g_Obj, BX + 8.0, BY + 2.0, BZ);
    if (!strcmp(s, "object_setrot")) return SetPlayerObjectRot(p, g_Obj, 0.0, 0.0, 45.0);
    if (!strcmp(s, "object_move")) return MovePlayerObject(p, g_Obj, BX + 8.0, BY + 2.0, BZ + 30.0, 1.0);
    if (!strcmp(s, "object_stop")) return StopPlayerObject(p, g_Obj);
    if (!strcmp(s, "object_material")) return SetPlayerObjectMaterial(p, g_Obj, 0, 10101, "2notherbuildsfe", "Bow_Abpave_Gen", 0xFF00FF00);
    if (!strcmp(s, "object_material_text")) return SetPlayerObjectMaterialText(p, g_Obj, "DIFF", 0, OBJECT_MATERIAL_SIZE_256x128, "Arial", 28, true, 0xFFFFFFFF, 0xFF000000, OBJECT_MATERIAL_TEXT_ALIGN_CENTER);
    if (!strcmp(s, "object_edit")) return EditPlayerObject(p, g_Obj);
    if (!strcmp(s, "object_select")) return SelectObject(p);
    if (!strcmp(s, "camera_attach_object")) return AttachCameraToPlayerObject(p, g_Obj);
    if (!strcmp(s, "object_attach_player")) return AttachPlayerObjectToPlayer(p, g_Obj, p, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0);
    if (!strcmp(s, "object_destroy")) return DestroyPlayerObject(p, g_Obj);

    // pickups (per-player pickup, model 1240 = health)
    if (!strcmp(s, "pickup_create"))
    {
        g_Pickup = CreatePlayerPickup(p, 1240, 1, BX + 5.0, BY - 3.0, BZ);
        return g_Pickup != -1;
    }
    if (!strcmp(s, "pickup_destroy")) return DestroyPlayerPickup(p, g_Pickup);

    // actors (streamed in/out by the server's streamer tick)
    if (!strcmp(s, "actor_create"))
    {
        g_Actor = CreateActor(101, BX + 6.0, BY + 3.0, BZ, 90.0);
        return g_Actor != INVALID_ACTOR_ID;
    }
    if (!strcmp(s, "actor_anim")) return ApplyActorAnimation(g_Actor, "DANCING", "dnce_M_a", 4.1, true, false, false, false, 0);
    if (!strcmp(s, "actor_anim_clear")) return ClearActorAnimations(g_Actor);
    if (!strcmp(s, "actor_facing")) return SetActorFacingAngle(g_Actor, 180.0);
    if (!strcmp(s, "actor_pos")) return SetActorPos(g_Actor, BX + 6.0, BY - 3.0, BZ);
    if (!strcmp(s, "actor_health")) return SetActorHealth(g_Actor, 50.0);
    if (!strcmp(s, "actor_destroy")) return DestroyActor(g_Actor);

    // vehicles (411 Infernus; 403 Linerunner + 435 trailer)
    if (!strcmp(s, "vehicle_create"))
    {
        g_Veh = CreateVehicle(411, BX + 7.0, BY + 6.0, BZ, 0.0, 1, 1, -1);
        return g_Veh != INVALID_VEHICLE_ID;
    }
    if (!strcmp(s, "vehicle_params_locked")) return SetVehicleParamsForPlayer(g_Veh, p, true, true);
    if (!strcmp(s, "vehicle_params_unlocked")) return SetVehicleParamsForPlayer(g_Veh, p, false, false);
    if (!strcmp(s, "vehicle_plate")) return SetVehicleNumberPlate(g_Veh, "DIFF");
    if (!strcmp(s, "vehicle_health")) return SetVehicleHealth(g_Veh, 400.0);
    if (!strcmp(s, "vehicle_damage")) return UpdateVehicleDamageStatus(g_Veh, VEHICLE_PANEL_STATUS:0x00030000, VEHICLE_DOOR_STATUS:0x00000202, VEHICLE_LIGHT_STATUS:0x5, VEHICLE_TYRE_STATUS:0x1);
    if (!strcmp(s, "vehicle_colour")) return ChangeVehicleColours(g_Veh, 3, 3);
    if (!strcmp(s, "vehicle_component_add")) return AddVehicleComponent(g_Veh, 1010);
    if (!strcmp(s, "vehicle_component_remove")) return RemoveVehicleComponent(g_Veh, 1010);
    if (!strcmp(s, "vehicle_zangle")) return SetVehicleZAngle(g_Veh, 90.0);
    if (!strcmp(s, "vehicle_pos")) return SetVehiclePos(g_Veh, BX + 7.0, BY + 8.0, BZ);
    if (!strcmp(s, "vehicle_interior_1")) return LinkVehicleToInterior(g_Veh, 1);
    if (!strcmp(s, "vehicle_interior_0")) return LinkVehicleToInterior(g_Veh, 0);
    if (!strcmp(s, "vehicle_put_player")) return PutPlayerInVehicle(p, g_Veh, 0);
    if (!strcmp(s, "vehicle_velocity")) return SetVehicleVelocity(g_Veh, 0.0, 0.3, 0.1);
    if (!strcmp(s, "vehicle_collisions_off")) return DisableRemoteVehicleCollisions(p, true);
    if (!strcmp(s, "vehicle_collisions_on")) return DisableRemoteVehicleCollisions(p, false);
    if (!strcmp(s, "vehicle_remove_player")) return RemovePlayerFromVehicle(p);
    if (!strcmp(s, "vehicle_create_truck_trailer"))
    {
        g_Truck = CreateVehicle(403, BX + 14.0, BY - 8.0, BZ + 0.5, 0.0, 1, 1, -1);
        g_Trailer = CreateVehicle(435, BX + 14.0, BY - 18.0, BZ + 0.5, 0.0, 1, 1, -1);
        return g_Truck != INVALID_VEHICLE_ID && g_Trailer != INVALID_VEHICLE_ID;
    }
    if (!strcmp(s, "trailer_attach")) return AttachTrailerToVehicle(g_Trailer, g_Truck);
    if (!strcmp(s, "trailer_detach")) return DetachTrailerFromVehicle(g_Truck);
    if (!strcmp(s, "vehicle_destroy_trailer")) return DestroyVehicle(g_Trailer);
    if (!strcmp(s, "vehicle_destroy_truck")) return DestroyVehicle(g_Truck);

    // remote player: an open.mp NPC (server-side, deterministic)
    if (!strcmp(s, "npc_create"))
    {
        g_Npc = NPC_Create("DiffNpc");
        return g_Npc != INVALID_PLAYER_ID;
    }
    if (!strcmp(s, "npc_spawn"))
    {
        NPC_Spawn(g_Npc);
        NPC_SetPos(g_Npc, BX + 4.0, BY + 3.0, BZ);
        NPC_SetFacingAngle(g_Npc, 90.0);
        return true;
    }
    if (!strcmp(s, "npc_chat_bubble")) return SetPlayerChatBubble(g_Npc, "diff bubble", 0xFFFFFFFF, 50.0, 10000);
    if (!strcmp(s, "npc_nametag_hide")) return ShowPlayerNameTagForPlayer(p, g_Npc, false);
    if (!strcmp(s, "npc_nametag_show")) return ShowPlayerNameTagForPlayer(p, g_Npc, true);
    if (!strcmp(s, "npc_colour_green")) return SetPlayerColor(g_Npc, 0x00FF00FF);
    if (!strcmp(s, "npc_put_in_vehicle")) return NPC_PutInVehicle(g_Npc, g_Veh, 0);
    if (!strcmp(s, "npc_exit_vehicle")) return NPC_ExitVehicle(g_Npc);
    if (!strcmp(s, "spectate_on")) return TogglePlayerSpectating(p, true);
    if (!strcmp(s, "spectate_npc")) return PlayerSpectatePlayer(p, g_Npc);
    if (!strcmp(s, "spectate_vehicle")) return PlayerSpectateVehicle(p, g_Veh);
    if (!strcmp(s, "spectate_off")) return TogglePlayerSpectating(p, false);
    if (!strcmp(s, "npc_kill")) { NPC_Kill(g_Npc); return true; }
    if (!strcmp(s, "npc_stream_out")) { NPC_SetVirtualWorld(g_Npc, 9); return true; }
    if (!strcmp(s, "npc_destroy")) { NPC_Destroy(g_Npc); g_Npc = INVALID_PLAYER_ID; return true; }
    if (!strcmp(s, "vehicle_destroy")) return DestroyVehicle(g_Veh);

    // spawn / class selection / session
    if (!strcmp(s, "spawn_immediately")) return SpawnPlayer(p);
    if (!strcmp(s, "remove_buildings")) return RemoveBuildingForPlayer(p, -1, BX + 30.0, BY, BZ, 25.0);
    if (!strcmp(s, "spawn_info")) return SetSpawnInfo(p, NO_TEAM, 60, BX, BY, BZ, BA);
    if (!strcmp(s, "force_class_selection")) return ForceClassSelection(p);
    if (!strcmp(s, "kill_to_class_selection")) return SetPlayerHealth(p, 0.0);
    if (!strcmp(s, "gamemode_restart")) { GameModeExit(); return true; }

    printf("[diff] error no implementation for %s", s);
    return false;
}
