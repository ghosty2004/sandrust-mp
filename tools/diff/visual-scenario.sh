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
    sleep 3; cap vehtag ui
    killbots
    # The chat input last: opening it resizes the chat GUI dialog for good.
    key t; typ "hello"; sleep 0.5; cap input ui
    key Escape; sleep 0.5
}
