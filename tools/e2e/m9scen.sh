#!/usr/bin/env bash
# M9 audio scenario, shared by tools/e2e/m9.sh (our client, lane "ui") and tools/e2e/m9-orig.sh
# (the original client, trace lane), so both clients get the identical input.
# The caller defines: fs 'CMD|…' (tools/trace/uifs.pwn command, waits until it ran), key KEY…,
# mark TEXT (tracer mark), and URL_BASE (http://127.0.0.1:PORT of tools/trace/icyserve.py).
# Every step starts with a mark; tools/trace/bass_diff.py compares the segments.

m9_silence_mp3() {
    # A silent MP3 (the test audio must stay inaudible anyway: Wine plays into a null sink).
    local f="$1"
    [[ -s "$f" ]] && return 0
    mkdir -p "$(dirname "$f")"
    ffmpeg -loglevel error -f lavfi -i anullsrc=r=44100:cl=stereo -t 30 -b:a 128k -y "$f"
}

m9_scenario() {
    fs 'pos|L|1700.0|-2600.0|13.5|90.0'
    sleep 3
    mark m9_global
    fs "audio|L|$URL_BASE/icyh.mp3|0.0|0.0|0.0|50.0|0"
    sleep 4
    mark m9_replace_near
    fs "audio|L|$URL_BASE/icyh.mp3|1700.0|-2600.0|13.5|50.0|1"
    sleep 3
    mark m9_pos20
    fs "audio|L|$URL_BASE/plain.mp3|1720.0|-2600.0|13.5|50.0|1"
    sleep 3
    mark m9_pos60
    fs "audio|L|$URL_BASE/plain.mp3|1760.0|-2600.0|13.5|50.0|1"
    sleep 3
    mark m9_global2
    fs "audio|L|$URL_BASE/icy.mp3|0.0|0.0|0.0|50.0|0"
    sleep 3
    mark m9_menu
    key Escape
    sleep 2
    key Escape
    sleep 2
    mark m9_stop
    fs 'audiostop|L'
    sleep 2
    mark m9_bad
    fs 'audio|L|http://127.0.0.1:9/none.mp3|0.0|0.0|0.0|50.0|0'
    sleep 2
    mark m9_bad_stop
    fs 'audiostop|L'
    sleep 2
    mark m9_sound_oneoff
    fs 'sound|L|1052|1700.0|-2600.0|13.5'
    sleep 1
    mark m9_sound_track
    fs 'sound|L|5|0.0|0.0|0.0'
    sleep 1
    mark m9_sound_zero
    fs 'sound|L|0|0.0|0.0|0.0'
    sleep 1
    mark m9_sound_mission
    fs 'sound|L|2000|1700.0|-2600.0|13.5'
    sleep 2
    mark m9_sound_zero2
    fs 'sound|L|0|0.0|0.0|0.0'
    sleep 1
    mark m9_end
}
