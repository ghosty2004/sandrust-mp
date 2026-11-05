#!/usr/bin/env bash
# FEAT-B visual elements (FEAT-8): `/dl` vehicle labels and `/testdw` death-window rows. Sourced by
# tools/diff/visual.sh through VISUAL_SCENARIO_FILE (same primitives as tools/diff/visual-scenario.sh), e.g.
#   VISUAL_SCENARIO_FILE=tools/diff/visual-featb.sh VISUAL_DIR=target/visual-featb tools/diff/visual.sh --orig
visual_scenario() {
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 2
    # A Landstalker 6 m east and an Infernus 12 m north (both inside the 20 m label range).
    fs 'veh|400|1706.0|-2600.0|13.5|0.0|1|1'
    fs 'veh|411|1700.0|-2588.0|13.5|90.0|1|1'
    sleep 3
    key t; typ "/dl"; key Return; sleep 1.5
    cap dl ui
    key t; typ "/dl"; key Return; sleep 1
    cap dloff ui
    key t; typ "/testdw"; key Return; sleep 1
    cap testdw ui
}
