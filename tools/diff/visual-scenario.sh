#!/usr/bin/env bash
# DIFF-4 scenario (sourced by tools/diff/visual.sh for BOTH clients). One capture per UI element:
# `cap NAME MODE [FRAMES]` logs FRAMES (default 3) Presents of the Direct3D call stream (tracer
# `d3d MODE`) to d3d-NAME.jsonl and takes NAME.png. The primitives (fs, key, kdown, kup, move,
# click, typ, cap, bot, waitlog, say) come from visual.sh and act on the client being driven
# (SIDE=orig|ours); the server runs tools/e2e/probe/m8afs.pwn. Keep both sides identical: the
# element diffs rely on the same inputs at the same moments.
visual_scenario() {
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    # Dialogs (TRACE-3 bodies, all six styles).
    fs 'dialog|10|0|Caption {FF0000}red|Line one\nLine {00FF00}two\tTabbed|OK|Cancel'
    sleep 1; cap dlg0 ui
    key Escape; waitlog 'dialog-response player [0-9]+ id 10 ' 5
    fs 'dialog|11|1|Input caption|Enter some text:|OK|Cancel'
    sleep 1; cap dlg1 ui
    typ "hi there"; key Return; waitlog 'id 11 response 1' 5
    fs 'dialog|13|3|Password|Secret:|OK|'
    sleep 0.5; typ "abc"; sleep 0.5; cap dlg3 ui
    key Return; waitlog 'id 13 response 1' 5
    fs 'dialog|20|2|List caption|Row 0\n{FF0000}Row 1 red\nRow {00FF00}2 green\n\nRow 4 after empty\n|Select|Close'
    sleep 1; cap dlg2 ui
    key Down; key Down; key Down; key Return; waitlog 'id 20 response 1' 5
    fs 'dialog|40|4|Tablist|a\tb\tc\nlong cell number one\t2\t3\n{00FF00}x\ty\tz\tw\tv\n\tlead\ttrail\t|OK|Cancel'
    sleep 1; cap dlg4 ui
    key Escape; waitlog 'id 40 response 0' 5
    fs 'dialog|50|5|Headers|H1\tHeader two\tH3\nr1c1\tr1c2\tr1c3\nr2\t\tr2c3|OK|Cancel'
    sleep 1; cap dlg5 ui
    key Down; key Return; waitlog 'id 50 response 1' 5
    # F1 help dialog (local; answering sends nothing).
    kdown F1; sleep 0.5; kup F1; sleep 0.5
    cap help ui
    key Return; sleep 0.5
    # Textdraws, sprite, selection.
    fs 'td|0|100.0|150.0|1|1|~g~M8A ~w~textdraw'
    fs 'td|1|320.0|300.0|2|2|Centred'
    fs 'td|2|500.0|20.0|4|1|LD_SPAC:white'
    fs 'tdsize|2|100.0|60.0'
    sleep 1; cap sprite all
    fs 'tdsel|2|1'
    fs 'select|FF0000FF'
    move 1100 80; sleep 0.5; cap spritesel ui
    click 1100 80; waitlog 'click-textdraw player [0-9]+ id [0-9]+' 5
    key Escape; waitlog 'click-textdraw player [0-9]+ id 65535' 5
    fs 'tdhide|2'
    sleep 1; cap textdraws all
    fs 'tdsel|0|1'
    fs 'select|FF0000FF'
    move 300 255; sleep 0.5; cap select ui
    key Escape; waitlog 'click-textdraw player [0-9]+ id 65535' 5
    fs 'tdhide|0'
    fs 'tdhide|1'
    # `mdlNNN:` sprite names take the generic TXD path in R4 (no custom-model store).
    fs 'td|3|500.0|20.0|4|1|mdl123:abc'
    fs 'tdsize|3|100.0|60.0'
    sleep 1; cap mdl all
    fs 'tdhide|3'
    # Menu.
    fs 'menu|Shop|200.0|150.0|200.0|0.0|Apple;Pear;Plum'
    fs 'menushow'
    sleep 1; cap menu all
    kdown Return; sleep 0.2; kup Return; waitlog 'menu-exit player' 6
    # Death window.
    fs 'death|-|L|200'
    fs 'death|L|L|24'
    sleep 0.5; cap dw ui
    # F10 / F5 held, F8 screenshot.
    kdown F10; sleep 0.5; cap f10 ui; kup F10; sleep 0.3
    kdown F5; sleep 1.2; cap netstats ui; kup F5; sleep 0.3
    cap_start f8 all 3; key F8; cap_end f8
    sleep 1; cap after-f8 ui
    # Clipboard into the chat input (Ctrl+V), then the input itself.
    clip 'clip text 1'
    key t; sleep 0.5; key ctrl+v; sleep 0.5; cap paste ui
    key Escape; sleep 0.5
    # Bot: nametag, bubble, scoreboard.
    bot Bot1 --seconds 200
    waitlog 'connect [0-9]+ Bot1' 30
    sleep 3
    fs 'pos|B1|1696.0|-2600.0|13.5|270.0'
    fs 'color|B1|00FF00FF'
    fs 'armour|B1|100'
    fs 'death|B1|L|31'
    fs 'bubble|B1|FFFF00FF|50.0|30000|Hello from the bot bubble'
    sleep 2; cap tags ui
    key Tab; sleep 4; cap scoreboard ui; key Tab; sleep 0.5
    killbots
    # Remote driver nametag (MR-M8A-5): a bot in a car next to us.
    fs 'veh|411|1696.0|-2592.0|13.5|0.0|1|1'
    bot Bot2 --mode drive --speed 0.0 --seconds 120
    waitlog 'spawn [0-9]+' 30
    sleep 2
    fs 'pos|B2|1694.0|-2592.0|13.5|0.0'
    waitlog 'state [0-9]+ 1->2' 30
    # The remote get-in animation and the vehicle's settling run for a few seconds after the
    # server's state change: capture the seated, static driver (tag anchor unchanged per frame).
    sleep 8; remote_probe vehtag; cap vehtag ui; remote_probe vehtag
    killbots
    # The chat input: opening it resizes the chat GUI dialog for good.
    key t; typ "hello"; sleep 0.5; cap input ui
    key Escape; sleep 0.5
    # FEAT-B (FEAT-8): `/dl` vehicle labels on and off, `/testdw` death-window rows. A Landstalker
    # 6 m east and an Infernus 12 m north (inside the 20 m label range; the vehtag Infernus stays).
    # Escape keeps the input's text ("clip text 1hello" from paste + input): send it as a chat
    # line first so the commands below start on an empty input.
    mark_log; key t; sleep 0.3; key Return; waitlog 'clip text 1hello' 5
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    fs 'veh|400|1706.0|-2600.0|13.5|0.0|1|1'
    fs 'veh|411|1700.0|-2588.0|13.5|90.0|1|1'
    sleep 3
    key t; typ "/dl"; key Return; sleep 1.5
    cap dl ui
    key t; typ "/dl"; key Return; sleep 1
    cap dloff ui
    key t; typ "/testdw"; key Return; sleep 1
    cap testdw ui
    visual_labels
    visual_mattext
    visual_previews
    visual_classdlg
}

# Object material text (RPC 44 + 84, user report "Romania Super Stunt"): m8afs `mtscene` creates 14 sign / board
# objects on a wall west of the player (text for six of them inside RPC 44), `mtmat` sets the rest (RPC 84):
# sizes 32x32..512x512, Arial / Verdana / Impact / Courier New / Tahoma, font sizes 20..80, bold 0/1, alignment
# 0/1/2, background 0 / opaque / half alpha, colour codes and newlines, texture + text on the same and on
# different indices.
# mattext: every render-target fill (logged in `ui` mode: they run outside GTA's scene). mattextw: a fixed camera
# on the wall; the render targets bound while the objects are drawn (+ mattextw.png, mattext2.png close-ups).
visual_mattext() {
    fs 'time|12|0'
    fs 'weather|1'
    fs 'pos|L|1722.0|-2600.0|13.5|90.0'
    sleep 2
    cap_start mattext ui 240
    fs 'mtscene|1700.0|-2607.0|14.0'
    sleep 1
    fs 'mtmat'
    sleep 1
    cap_end mattext
    fs 'togglecontrol|L|0'
    fs 'campos|L|1721.0|-2600.25|21.0'
    fs 'camlook|L|1700.0|-2600.25|21.0'
    sleep 3
    cap mattextw all 2
    fs 'campos|L|1709.0|-2600.25|16.0'
    fs 'camlook|L|1700.0|-2600.25|16.0'
    sleep 2
    shot mattext2
    fs 'camera|L'
    fs 'togglecontrol|L|1'
}

# Model-preview textdraws (font 5, A 0x100b34e0 from the game-frame hook after CGame::Process): one off-screen
# RenderWare render per textdraw into its own 256x256 camera-texture raster, then the sprite path draws that texture.
# pv* captures log every call (`all`) of the frames around one creation (the render-target pass); mdlpreview /
# mdlpvupd / mdlpvreset / mdlpvdel are the steady display (kind `preview`: the textured quads). Ped (skin 7), vehicle
# (411, colours 3/6), SAMP.img object (19320 pumpkin, transparent background), a box with zoom 2, a weapon with zoom
# 0.6 and a letter colour, a negative model (-5: samp's 18631 fallback, turned 180 degrees; an id of 20000 reads past
# GTA's model table and crashes samp), a model outside the ped, vehicle and object ranges (315: never rendered, stays
# on the text path), a player textdraw (skin 162: its own height); then a rotation update and a model change while
# shown (re-sent RPC 134), a device loss, then destroy / hide.
visual_previews() {
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    cap_start pvped all 48
    fs 'tdprev|4|20.0|160.0|90.0|110.0|7|-10.0|0.0|-20.0|1.0|336699FF|-1|-1'
    cap_end pvped; shot pvped
    cap_start pvveh all 48
    fs 'tdprev|5|120.0|160.0|120.0|90.0|411|-15.0|0.0|-40.0|1.0|202020FF|3|6'
    cap_end pvveh; shot pvveh
    cap_start pvobj all 48
    fs 'tdprev|6|250.0|160.0|80.0|80.0|19320|0.0|0.0|30.0|1.0|00000000|-1|-1'
    cap_end pvobj; shot pvobj
    # One render per frame: a pause after each show, so the RPCs never arrive in the same frame (two renders in one
    # frame share one scene; the split depends on network timing).
    cap_start pvmany all 400
    fs 'tdprev|7|340.0|160.0|70.0|70.0|1220|20.0|10.0|45.0|2.0|AA3300C0|-1|-1'; sleep 0.6
    fs 'tdprev|8|420.0|160.0|70.0|70.0|348|0.0|0.0|90.0|0.6|FFFFFF40|-1|-1|FF8080FF'; sleep 0.6
    fs 'tdprev|9|500.0|160.0|70.0|70.0|-5|0.0|0.0|0.0|1.0|000000FF|-1|-1'; sleep 0.6
    fs 'tdprev|3|500.0|260.0|70.0|70.0|315|0.0|0.0|0.0|1.0|000000FF|-1|-1'; sleep 0.6
    fs 'ptdprev|20.0|300.0|60.0|80.0|162|0.0|0.0|180.0|1.0|404040FF'
    cap_end pvmany
    sleep 1; cap mdlpreview all
    cap_start pvrot all 48
    fs 'tdprot|5|-15.0|0.0|140.0|1.5'
    cap_end pvrot
    cap_start pvmodel all 48
    fs 'tdpmodel|4|162'
    cap_end pvmodel
    sleep 1; cap mdlpvupd all
    # Device loss + Reset (tracer `d3d devreset`): samp frees every preview texture in its lost-device pass and the
    # next game frame renders them again.
    cap_start pvreset all 150
    tctl "d3d devreset 15" > /dev/null
    cap_end pvreset
    sleep 1; cap mdlpvreset all
    fs 'tddel|5'
    fs 'tdhide|6'
    fs 'ptddel'
    sleep 1; cap mdlpvdel all
    local i
    for i in 3 4 7 8 9; do fs "tddel|$i"; done
    sleep 0.5
}

# Class selection with a dialog (samp A 0x10006100, run from the netgame tick while selecting a class): the
# class-selection panel (GUI dialog 0x1026eaa4) is made visible on the first such frame WITHOUT an open dialog
# (dialog +0x28); while a dialog is open nothing happens, so a dialog that is already open when the class selection
# starts keeps the panel away until it closes. A dialog opened over a shown panel does not hide it.
visual_classdlg() {
    fs 'dialog|60|0|Language|Choose your language:\nEnglish / Romana|OK|Cancel'
    sleep 1
    fs 'forceclass|L'
    # The gamemode (gungame) spectates the dead player; leaving spectating with the force flag set goes to the class
    # selection (with the dialog still open).
    fs 'kill|L'
    sleep 4
    fs 'unspec|L'
    # The first RPC 128 waits for the dialog too (it is sent on the panel's first frame, A 0x100058d0).
    sleep 4
    cap classdlg ui
    key Escape; waitlog 'id 60 response 0' 5
    waitlog '\[e2e\] class id=' 10
    sleep 2
    cap classpanel ui
    fs 'dialog|61|0|Second|Opened over the panel|OK|'
    sleep 1
    cap classdlg2 ui
    key Return; waitlog 'id 61 response 1' 5
    sleep 0.5
    local i
    for i in 1 2 3; do
        mark_log
        key shift
        waitlog "\[e2e\] spawn id=" 8 && break
    done
    sleep 2
}

# 3D text labels (RPC 36) and chat bubbles with `{RRGGBB}` codes: the pool pass A 0x10001350 and the bubble pass
# A 0x10066ed0 both draw through the 3D text painter A 0x1006bc90 (outline passes of the stripped text, then the raw
# text through the colour-code wrapper). label3d: world labels in front of the local player (several colours, a code
# at the start and mid-word, invalid codes, lower-case hex, base alpha < FF with and without codes, a 432-char text
# with a literal `~k~`, a short `~k~` text that is expanded). bubblecol: a bot with a coloured bubble (alpha 80)
# and a per-player label attached to it.
visual_labels() {
    local long="" i cols=(FF0000 00FF00 0000FF FFFF00 FF00FF 00FFFF FFFFFF)
    for ((i = 0; i < 9; i++)); do long+="{${cols[i % 7]}}line $i of the long label, colour ${cols[i % 7]}\\n"; done
    long+="~k~~PED_FIREWEAPON~ stays literal"
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    fs 'label|{FF0000}red {00FF00}green {0000FF}blue {FFFFFF}white|FFFF00FF|1692.0|-2600.0|14.6|50.0|0'
    fs 'label|plain yel{FF00FF}low mid-word|FFFF00FF|1692.0|-2597.0|14.1|50.0|0'
    fs 'label|bad {GGGGGG}code {12345}short {ff8800}lower {00FF00}{0000FF}double|00FFFFFF|1692.0|-2603.0|14.1|50.0|0'
    fs 'label|half alpha {FF0000}then red|FFFFFF80|1692.0|-2600.0|13.6|50.0|0'
    fs 'label|alpha without codes|00FF0060|1692.0|-2602.0|13.1|50.0|0'
    fs 'label|press ~k~~PED_FIREWEAPON~ {FF0000}now|FFFFFFFF|1692.0|-2598.0|13.1|50.0|0'
    fs "label|$long|FFFFFFFF|1680.0|-2600.0|19.0|80.0|0"
    sleep 1.5
    cap label3d ui
    bot Bot3 --seconds 120
    waitlog 'connect [0-9]+ Bot3' 30
    sleep 3
    local b="B$BOTCOUNT"
    fs "pos|$b|1696.0|-2600.0|13.5|270.0"
    fs "bubble|$b|FFFF0080|50.0|30000|{FF0000}Red {00FF00}green {FFFFFF}white bubble"
    fs "plabel|L|attached {00FFFF}cyan {FF8800}orange|FFFFFFFF|0.0|0.0|0.6|50.0|$b|-|0"
    sleep 2
    cap bubblecol ui
    killbots
}
