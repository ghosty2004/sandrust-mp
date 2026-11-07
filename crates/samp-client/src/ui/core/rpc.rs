//! S→C RPCs the UI state handles: chat and client messages, game text, joins and quits, name tags, chat
//! bubbles, dialogs, textdraws, menus, scores, death messages, names and colours.

use samp_proto::rpcs::{Incoming, player, remote, session, ui};
use samp_ui::chat::ClientMessageResult;
use samp_ui::dialog::ShowOutcome;
use samp_ui::gametext::GameText;
use samp_ui::text::KeyNameExpander;

use super::{CursorRequest, INVALID_PLAYER, MenuRequest, ShownGameText, UiCore, local_time_now, tick_ms};

impl UiCore {
    // ---- RPCs --------------------------------------------------------------------

    /// Every decoded S→C RPC passes through here (before the game handlers).
    pub fn on_rpc(&mut self, r: &Incoming, expander: &dyn KeyNameExpander) {
        match r {
            Incoming::Ui(ui::Incoming::SendClientMessage(m)) => self.on_client_message(m, expander),
            Incoming::Ui(ui::Incoming::PlayerChatMessage(m)) => self.on_player_chat(m.player_id, &m.message),
            Incoming::Ui(ui::Incoming::SendGameText(g)) => self.on_game_text(g),
            Incoming::Remote(remote::Incoming::PlayerJoin(j)) => self.on_player_join(j),
            Incoming::Remote(remote::Incoming::PlayerQuit(q)) => self.on_player_quit(q),
            Incoming::Remote(remote::Incoming::ShowPlayerNameTagForPlayer(t)) => self.on_name_tag_shown(t),
            Incoming::Remote(remote::Incoming::SetPlayerChatBubble(b)) => self.on_chat_bubble(b),
            Incoming::Ui(ui::Incoming::ShowDialog(d)) => self.on_show_dialog(d),
            Incoming::Ui(ui::Incoming::PlayerShowTextDraw(t)) => self.on_show_textdraw(t),
            Incoming::Ui(ui::Incoming::PlayerHideTextDraw(t)) => self.textdraws.hide(t.textdraw_id),
            Incoming::Ui(ui::Incoming::PlayerTextDrawSetString(t)) => {
                self.textdraws.set_string(t.textdraw_id, &t.text);
            }
            Incoming::Ui(ui::Incoming::PlayerBeginTextDrawSelect(s)) => self.on_begin_textdraw_select(s),
            Incoming::Ui(ui::Incoming::PlayerInitMenu(m)) => {
                self.menus.init(m);
            }
            Incoming::Ui(ui::Incoming::PlayerShowMenu(m)) => self.on_show_menu(m),
            Incoming::Ui(ui::Incoming::PlayerHideMenu(m)) => self.on_hide_menu(m),
            Incoming::Session(session::Incoming::SendPlayerScoresAndPings(s)) => self.on_scores(s),
            Incoming::Player(player::Incoming::SendDeathMessage(d)) => self.on_death_message(d),
            Incoming::Remote(remote::Incoming::SetPlayerName(n)) => self.on_set_player_name(n),
            // Only for the local id or a known remote id (player-state.md, "72 SetPlayerColor").
            Incoming::Player(player::Incoming::SetPlayerColor(c))
                if Some(c.player_id) == self.local_id || self.names.contains_key(&c.player_id) =>
            {
                self.colours.set(c.player_id, c.colour);
            }
            _ => {}
        }
    }

    /// RPC 93 SendClientMessage: a chat line in the message colour, `~k~` key names expanded.
    fn on_client_message(&mut self, m: &ui::SendClientMessage, expander: &dyn KeyNameExpander) {
        if let ClientMessageResult::Added(l) =
            self.chat.add_client_message(m.colour, &m.message, expander, local_time_now())
        {
            self.log_lines(l);
        }
    }

    /// RPC 73 SendGameText: an accepted text is remembered and handed to the host.
    fn on_game_text(&mut self, g: &ui::SendGameText) {
        match GameText::from_rpc(g) {
            GameText::Show { style, time_ms, text } => {
                self.last_game_text = Some(ShownGameText { style: style as u8, time_ms, text: text.clone() });
                self.requests.game_text = Some((style as u8, time_ms, text));
            }
            other => crate::log!("ui: game text ignored ({other:?})"),
        }
    }

    /// RPC 137 PlayerJoin: the name, and the colour when it is not zero.
    fn on_player_join(&mut self, j: &remote::PlayerJoin) {
        if j.name.len() <= samp_ui::chat::MAX_PREFIX_LEN && j.player_id < 1005 {
            self.names.insert(j.player_id, j.name.clone());
            // A non-zero colour sets the table entry for ids < 1004 (remote-players-sync.md,
            // "137 PlayerJoin").
            if j.colour != 0 && j.player_id < 1004 {
                self.colours.set(j.player_id, j.colour);
            }
        }
    }

    /// RPC 138 PlayerQuit: everything held for the player goes.
    fn on_player_quit(&mut self, q: &remote::PlayerQuit) {
        self.names.remove(&q.player_id);
        self.bubbles.clear(q.player_id);
        self.hidden_tags.remove(&q.player_id);
        self.scores.remove(&q.player_id);
    }

    /// RPC 80 ShowPlayerNameTagForPlayer.
    fn on_name_tag_shown(&mut self, t: &remote::ShowPlayerNameTagForPlayer) {
        if t.show != 0 {
            self.hidden_tags.remove(&t.player_id);
        } else {
            self.hidden_tags.insert(t.player_id);
        }
    }

    /// RPC 59 SetPlayerChatBubble: only for a known player.
    fn on_chat_bubble(&mut self, b: &remote::SetPlayerChatBubble) {
        if self.names.contains_key(&b.player_id) {
            self.bubbles.set(b.player_id, b.colour, b.draw_distance, b.expire_time, &b.text, tick_ms());
        }
    }

    /// RPC 61 ShowDialog: opening closes the chat input; closing an open dialog asks for cursor mode 0.
    fn on_show_dialog(&mut self, d: &ui::ShowDialog) {
        match self.dialogs.show(d) {
            ShowOutcome::Opened => {
                self.input.close();
                self.dialog_serial += 1;
            }
            ShowOutcome::Closed { was_open: true } => {
                self.requests.cursor.push(CursorRequest::Mode0 { immediate: false })
            }
            _ => {}
        }
    }

    /// RPC 134 PlayerShowTextDraw: texts at or over the limit are refused with a warning line.
    fn on_show_textdraw(&mut self, t: &ui::PlayerShowTextDraw) {
        if t.text.len() >= samp_ui::textdraw::TEXT_MAX {
            self.print_debug(format!("Warning: ignoring large TextDraw size={}", t.text.len()).as_bytes());
        } else {
            self.textdraws.show(t.textdraw_id, t.data, &t.text);
        }
    }

    /// RPC 83 PlayerBeginTextDrawSelect.
    fn on_begin_textdraw_select(&mut self, s: &ui::PlayerBeginTextDrawSelect) {
        if s.enable {
            self.textdraws.begin_select(s.hover_colour);
        } else {
            self.cancel_textdraw_select();
        }
    }

    /// RPC 77 PlayerShowMenu.
    fn on_show_menu(&mut self, m: &ui::PlayerShowMenu) {
        if let Some((delete_previous, build)) = self.menus.show(m.menu_id) {
            self.requests.menus.push(MenuRequest::Show { delete_previous, build });
        }
    }

    /// RPC 78 PlayerHideMenu.
    fn on_hide_menu(&mut self, m: &ui::PlayerHideMenu) {
        if self.menus.hide(m.menu_id) {
            self.requests.menus.push(MenuRequest::Delete);
        }
    }

    /// RPC 155 SendPlayerScoresAndPings: records for ids below 1004.
    fn on_scores(&mut self, s: &session::SendPlayerScoresAndPings) {
        for p in &s.players {
            if p.player_id < 1004 {
                self.scores.insert(p.player_id, (p.score, p.ping));
            }
        }
    }

    /// RPC 11 SetPlayerName: the local name or a known remote name.
    fn on_set_player_name(&mut self, n: &remote::SetPlayerName) {
        if Some(n.player_id) == self.local_id {
            self.local_name = n.name.clone();
        } else if let Some(e) = self.names.get_mut(&n.player_id) {
            *e = n.name.clone();
        }
    }

    /// RPC 55 SendDeathMessage: names and colours of the ids; victim 0xFFFF drops it.
    fn on_death_message(&mut self, d: &player::SendDeathMessage) {
        if d.victim_id == INVALID_PLAYER {
            return;
        }
        let who = |u: &Self, id: u16| -> (Vec<u8>, u32) {
            match u.name(id) {
                Some(n) if id != INVALID_PLAYER => (n.to_vec(), u.colours.chat_argb(id)),
                _ => (Vec::new(), 0),
            }
        };
        let (kn, kc) = who(self, d.killer_id);
        let (vn, vc) = who(self, d.victim_id);
        self.deaths.add(&kn, kc, &vn, vc, d.reason);
    }

    /// RPC 101 PlayerChatMessage: only in netgame state 5; the local id uses the local name, other ids
    /// need an active remote slot (< 1005).
    fn on_player_chat(&mut self, id: u16, text: &[u8]) {
        if !self.connected {
            return;
        }
        let name = if Some(id) == self.local_id {
            self.local_name.clone()
        } else if id < 1005
            && let Some(n) = self.names.get(&id)
        {
            n.clone()
        } else {
            return;
        };
        let colour = self.colours.chat_argb(id);
        let l = self.chat.add_player_chat(&name, colour, text, local_time_now());
        self.log_lines(l);
    }
}
