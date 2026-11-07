//! Network-session hooks called from `client.rs`: session events, RPCs, the per-tick state the UI needs,
//! chat lines from other subsystems, and sending the UI's queued messages.

use std::sync::Mutex;

use samp_proto::raknet::Reliability;
use samp_proto::rpcs::Incoming;
use samp_proto::rpcs::session::OnPlayerRequestScoresAndPings;
use samp_proto::rpcs::ui::{
    OnPlayerExitedMenu, OnPlayerSelectTextDraw, OnPlayerSelectedMenuRow, PlayerRequestChatMessage,
    PlayerRequestCommandMessage,
};
use samp_proto::sync::rcon::RconCommand;

use super::adapters::GameExpander;
use super::{State, try_with_ui, with_ui};
use crate::netclient::NetClient;
use crate::ui::core::Outgoing;

/// The netgame is created (Client::start).
pub fn on_netgame_start(host: &str, port: u16, name: &str) {
    with_ui(|st| st.core.on_netgame_start(host, port, name.as_bytes()));
}

/// The configured frame limit (`fpslimit`, 20..90), if the UI exists and is not busy.
pub fn fps_limit() -> Option<u32> {
    try_with_ui(|st| st.core.settings.fps_limit).flatten()
}

/// Spawn: the mode-0 request that ends class selection's input lock uses the delayed (10-frame) restore.
pub fn class_selection_release() {
    let _ = with_ui(|st| st.spawn_release = true);
}

/// A type-8 (debug colour) chat line, e.g. samp's "Warning(…)" lines.
pub fn print_debug(text: &str) {
    with_ui(|st| st.core.print_debug(text.as_bytes()));
}

/// Debug chat lines raised while the UI state is locked (renderer construction, the object pre-step): printed
/// at the start of the next frame entry, in order.
static PENDING_DEBUG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Queues a debug chat line for the next frame entry (safe while the UI state is locked).
pub fn print_debug_later(text: String) {
    PENDING_DEBUG.lock().unwrap_or_else(|e| e.into_inner()).push(text);
}

pub(super) fn flush_pending_debug(st: &mut State) {
    let lines = std::mem::take(&mut *PENDING_DEBUG.lock().unwrap_or_else(|e| e.into_inner()));
    for l in lines {
        st.core.print_debug(l.as_bytes());
    }
}

/// Whether the `/ctd` camera-target debug labels are on.
pub fn camera_target_labels_on() -> bool {
    try_with_ui(|st| st.core.settings.camera_target_labels).unwrap_or(false)
}

/// `/ctd` labels, drawn from the local tick inside GTA's scene like samp's.
pub fn draw_camera_target_labels(items: &[([f32; 16], Vec<u8>)]) {
    let _ = try_with_ui(|st| {
        if st.core.settings.camera_target_labels
            && let Some(r) = st.renderer.as_ref()
        {
            r.camera_target_labels(items);
        }
    });
}

/// A type-4 (info colour) chat line.
pub fn print_info(text: &[u8]) {
    with_ui(|st| st.core.print_info(text));
}

/// `audiomsgoff` (sa-mp.cfg / `/audiomsg`).
pub fn audio_messages_off() -> bool {
    with_ui(|st| st.core.settings.audio_messages_off).unwrap_or(false)
}

/// Whether the `/dl` vehicle debug labels are on.
pub fn vehicle_labels_on() -> bool {
    try_with_ui(|st| st.core.settings.vehicle_labels).unwrap_or(false)
}

/// `disableheadmove` (sa-mp.cfg / `/headmove`): the player's head does not follow the camera.
pub fn head_move_disabled() -> bool {
    try_with_ui(|st| st.core.settings.head_move_disabled).unwrap_or(false)
}

/// RPC 139 InitGame.
pub fn on_init_game(g: &samp_proto::session::InitGame) {
    with_ui(|st| {
        st.net_stats.send_rate = g.on_foot_rate;
        // samp's lag-compensation selector: InitGame value 1 → 0, 2 → 1, anything else → 2.
        st.net_stats.shot_comp_mode = u32::from(crate::combat::lag_comp_selector(g.lag_compensation));
        st.core.on_init_game(g)
    });
}

/// RPC 40 / lost-connection reset: pools and server settings.
pub fn on_reset() {
    with_ui(|st| st.core.on_reset());
}

/// RPC 40 reset: the player colour table back to the defaults. Mirrors samp.dll 0x100A72C0.
pub fn reset_colours() {
    with_ui(|st| st.core.colours = crate::ui::colours::PlayerColours::new());
}

/// Lost connection: every remote player is deleted from the pool.
pub fn forget_players() {
    with_ui(|st| st.core.forget_players());
}

/// Every S→C RPC, before the game handlers.
pub fn on_rpc(r: &Incoming) {
    with_ui(|st| st.core.on_rpc(r, &GameExpander));
}

/// The pool block of the F5 overlay, once per session tick: vehicles in the pool, distinct models among those
/// with a GTA vehicle, remote ped slots in use, objects in the pool.
pub fn set_pool_stats(vehicles: u32, vehicle_models: u32, ped_slots: u32, objects: u32) {
    let _ = try_with_ui(|st| {
        let s = &mut st.net_stats;
        s.vehicles = vehicles;
        s.vehicle_models = vehicle_models;
        s.ped_slots_used = ped_slots;
        s.object_slots_used = objects;
    });
}

/// Remote players for nametags, bubbles and the scoreboard (once per session tick).
pub fn set_remotes(remotes: Vec<crate::ui::core::RemoteUi>) {
    let _ = try_with_ui(|st| st.remotes = remotes);
}

/// Class selection state (the scoreboard does not open during it).
pub fn set_class_selection(active: bool) {
    let _ = try_with_ui(|st| st.core.class_selection = active);
}

/// A dialog is open; class selection's tick does nothing meanwhile.
pub fn dialog_open() -> bool {
    with_ui(|st| st.core.dialogs.is_open()).unwrap_or(false)
}

/// The chat input is open; Shift then does not request a spawn.
pub fn chat_input_open() -> bool {
    with_ui(|st| st.core.input.is_open()).unwrap_or(false)
}

/// Sends the UI's queued messages now. The original sends them from its window procedure (a
/// dialog's RPC 62 when Enter is released, chat lines, clicks), i.e. before the frame's local
/// tick: the session tick calls this before the local player's part, `net_tick` again at its end.
pub fn send_outgoing(net: &mut NetClient) {
    let out = with_ui(|st| st.core.take_outgoing()).unwrap_or_default();
    for o in out {
        send_one(net, o);
    }
}

/// Once per session tick: connection lines and the UI's outgoing messages.
pub fn net_tick(net: &mut NetClient) {
    let c = net.counters();
    let l = net.link_stats().unwrap_or_default();
    let _ = with_ui(|st| {
        st.net_bytes = (c.bytes_sent, c.bytes_received);
        let s = &mut st.net_stats;
        s.messages_in_send_buffer = l.messages_in_send_buffer;
        s.messages_sent = l.messages_sent as u32;
        s.bytes_sent = (l.bits_sent / 8) as u32;
        s.acks_sent = l.acks_sent as u32;
        s.acks_in_send_buffer = l.acks_in_send_buffer;
        s.messages_waiting_for_ack = l.messages_waiting_for_ack;
        s.messages_resent = l.messages_resent as u32;
        s.bytes_resent = (l.bits_resent / 8) as u32;
        s.packet_loss = if l.bits_sent == 0 { 0.0 } else { l.bits_resent as f32 * 100.0 / l.bits_sent as f32 };
        s.messages_received = l.messages_received as u32;
        s.bytes_received = c.bytes_received as u32;
        s.acks_received = l.acks_received as u32;
        s.duplicate_acks_received = l.duplicate_acks_received as u32;
    });
    send_outgoing(net);
}

fn send_one(net: &mut NetClient, o: Outgoing) {
    match o {
        Outgoing::Chat(message) => {
            crate::log!("ui: RPC 101 {:?}", String::from_utf8_lossy(&message));
            net.send_rpc(&PlayerRequestChatMessage { message }, Reliability::Reliable);
        }
        Outgoing::Command(command) => {
            crate::log!("ui: RPC 50 {:?}", String::from_utf8_lossy(&command));
            net.send_rpc(&PlayerRequestCommandMessage { command }, Reliability::Reliable);
        }
        Outgoing::Rcon(command) => net.send_sync(&RconCommand { command }),
        Outgoing::DialogResponse(r) => {
            crate::log!(
                "ui: RPC 62 id={} response={} listitem={} text={:?}",
                r.dialog_id,
                r.response,
                r.list_item,
                String::from_utf8_lossy(&r.text)
            );
            net.send_rpc(&r, Reliability::ReliableOrdered);
        }
        Outgoing::SelectTextDraw(textdraw_id) => {
            crate::log!("ui: RPC 83 textdraw {textdraw_id:#x}");
            net.send_rpc(&OnPlayerSelectTextDraw { textdraw_id }, Reliability::ReliableOrdered);
        }
        Outgoing::MenuRow(row) => {
            crate::log!("ui: RPC 132 row {row}");
            net.send_rpc(&OnPlayerSelectedMenuRow { row }, Reliability::Reliable);
        }
        Outgoing::MenuExit => {
            crate::log!("ui: RPC 140");
            net.send_rpc(&OnPlayerExitedMenu, Reliability::Reliable);
        }
        Outgoing::ScoresRequest => net.send_rpc(&OnPlayerRequestScoresAndPings, Reliability::Reliable),
        Outgoing::ClickPlayer(player_id) => {
            crate::log!("ui: RPC 23 player {player_id}");
            let p = samp_proto::rpcs::session::OnPlayerClickPlayer {
                player_id,
                source: samp_proto::rpcs::session::OnPlayerClickPlayer::SOURCE_SCOREBOARD,
            };
            net.send_rpc(&p, Reliability::ReliableOrdered);
        }
        Outgoing::ClickMap(pos) => {
            crate::log!("ui: RPC 119 {pos:?}");
            let p = samp_proto::rpcs::session::OnPlayerClickMap {
                pos: samp_proto::types::Vec3 { x: pos[0], y: pos[1], z: pos[2] },
            };
            net.send_rpc(&p, Reliability::Reliable);
        }
    }
}
