//! Input on the UI state: routed key and mouse actions, edit-box keys and the clipboard, Enter in the chat
//! input with its local commands, dialog answers, the scoreboard and textdraw selection.

use std::io::Write;
use std::path::PathBuf;

use samp_ui::commands::{self, Effect, GameInfo};
use samp_ui::dialog::Button;
use samp_ui::edit::{CharResult, EditKey, Mods};
use samp_ui::input::Submit;
use samp_ui::keys::{self, Action, Event, UiState};
use samp_ui::scoreboard::PlayerInfo;

use super::{CursorRequest, Outgoing, SUBMIT_SCROLL_POS, UiCore, text_mode};

impl UiCore {
    /// The textdraw-selection disable path: mode off, cursor mode 0, chat input closed, RPC 83 with
    /// 0xFFFF (ui-chat-audio.md, "83 PlayerBeginTextDrawSelect").
    pub fn cancel_textdraw_select(&mut self) {
        self.textdraws.cancel_select();
        self.requests.cursor.push(CursorRequest::Mode0 { immediate: true });
        self.input.close();
        self.outgoing.push(Outgoing::SelectTextDraw(samp_ui::textdraw::NONE));
    }

    /// Answers the open dialog with RPC 62 and closes it (cursor mode 0, 10-frame delay).
    pub fn dialog_respond(&mut self, button: Button) {
        if !self.dialogs.is_open() {
            return;
        }
        if let Some(r) = self.dialogs.respond(button) {
            self.responses_sent += 1;
            self.outgoing.push(Outgoing::DialogResponse(r));
        }
        self.requests.cursor.push(CursorRequest::Mode0 { immediate: false });
    }

    /// Scoreboard rows (local first) from the names, colours and RPC 155 records.
    pub fn rebuild_scoreboard(&mut self) {
        let info = |u: &Self, id: u16, name: &[u8]| {
            let (score, ping) = u.scores.get(&id).copied().unwrap_or((0, 0));
            PlayerInfo { id, name: name.to_vec(), score, ping, colour_rgba: u.colours.rgba(id), is_npc: false }
        };
        let local = info(self, self.local_id.unwrap_or(0), &self.local_name);
        let remotes: Vec<PlayerInfo> = self.names.iter().map(|(&id, n)| info(self, id, n)).collect();
        self.scoreboard.rebuild(&local, &remotes);
        // Like samp, the first row (the local player) starts selected.
        if self.scoreboard.selected().is_none() && !self.scoreboard.rows().is_empty() {
            self.scoreboard.set_selected(Some(0));
        }
    }

    // ---- input -------------------------------------------------------------------

    /// Routing state for `samp_ui::keys::route`.
    pub fn key_state(&self, gta_menu: bool) -> UiState {
        let d = self.dialogs.current();
        UiState {
            textdraw_select: self.textdraws.selection.active,
            dialog_is_list: d.is_some_and(|d| d.style.is_list()),
            input_exists: self.netgame,
            input_open: self.input.is_open(),
            input_rect: self.input_rect,
            dialog_open: d.is_some(),
            dialog_has_edit: d.is_some_and(|d| d.style.has_edit()),
            scoreboard_exists: self.netgame,
            scoreboard_open: self.scoreboard.is_open(),
            scoreboard_rect: self.scoreboard_rect,
            class_selection_visible: false,
            // Object edit / selection: an ESC key-down is swallowed so GTA does not open its pause menu.
            other_modal: crate::objedit::with(|sh| sh.editor.active || sh.selection.active),
            gta_menu,
            ime_composing: false,
            netgame: self.netgame,
            chat_mode: self.chat.mode(),
            debug_mode: self.debug_mode,
        }
    }

    /// Applies routed actions.
    pub fn apply(&mut self, actions: &[Action], game: &dyn GameInfo, wheel_lines: u32) {
        for &a in actions {
            match a {
                Action::OpenInput => {
                    self.input.open();
                }
                Action::CloseInput => self.input.close(),
                Action::InputChar(0x16) => self.requests.paste = true,
                Action::InputChar(c) => match self.input.char(c) {
                    CharResult::Enter => self.submit(game),
                    CharResult::Copy(sel) => self.copy(sel),
                    CharResult::Edited | CharResult::Ignored => {}
                },
                Action::SubmitInput => self.submit(game),
                Action::HistoryOlder => self.input.history_older(),
                Action::HistoryNewer => self.input.history_newer(),
                Action::ChatPageUp => self.chat.page_up(),
                Action::ChatPageDown => self.chat.page_down(),
                Action::ChatWheel(n) => self.chat.wheel(n, wheel_lines),
                Action::CycleChatMode => {
                    self.chat.cycle_mode();
                }
                Action::InputRestore => self.requests.input_restore = true,
                Action::DialogChar(0x16) => self.requests.paste = true,
                Action::DialogChar(c) => {
                    if let Some(d) = self.dialogs.current_mut() {
                        // Enter in the edit box answers with button 1.
                        match d.edit.char(c) {
                            CharResult::Enter => self.dialog_respond(Button::Ok),
                            CharResult::Copy(sel) => self.copy(sel),
                            _ => {}
                        }
                    }
                }
                Action::DialogListKey(k) => {
                    if let Some(d) = self.dialogs.current_mut() {
                        d.list_key(k, self.dialog_page_rows);
                    }
                }
                Action::DialogResponse(b) => self.dialog_respond(b),
                Action::CancelTextDrawSelect => self.cancel_textdraw_select(),
                Action::ToggleScoreboard => {
                    let was = self.scoreboard.is_open();
                    let open = self.scoreboard.toggle(self.class_selection);
                    if open && !was {
                        self.rebuild_scoreboard();
                        self.requests.cursor.push(CursorRequest::Mode3);
                    } else if was && !open {
                        self.requests.cursor.push(CursorRequest::Mode0 { immediate: true });
                    }
                }
                Action::CloseScoreboard => {
                    if self.scoreboard.is_open() {
                        self.scoreboard.close();
                        self.requests.cursor.push(CursorRequest::Mode0 { immediate: true });
                    }
                }
                Action::ToggleDeathWindow => {
                    self.deaths.toggle_visible();
                }
                Action::Screenshot => self.requests.screenshot = true,
                Action::CursorModeZeroImmediate => self.requests.cursor.push(CursorRequest::Mode0 { immediate: true }),
                other => crate::log!("ui: action {other:?} not handled"),
            }
        }
    }

    /// Edit-box key-down while the input or a dialog edit box has focus (DXUT edit box).
    pub fn edit_key(&mut self, key: EditKey, mods: Mods) -> bool {
        let copied = if self.input.is_open() {
            self.input.key(key, mods)
        } else if let Some(d) = self.dialogs.current_mut()
            && d.style.has_edit()
            && !self.scoreboard.is_open()
        {
            d.edit.key(key, mods)
        } else {
            return false;
        };
        if let Some(sel) = copied {
            self.copy(sel);
        } else if key == EditKey::Insert && mods.shift && !mods.ctrl {
            self.requests.paste = true;
        }
        true
    }

    /// DXUT `CopyToClipboard`: only when something is selected.
    fn copy(&mut self, sel: Vec<u8>) {
        if !sel.is_empty() {
            self.requests.clipboard_copy = Some(sel);
        }
    }

    /// DXUT `PasteFromClipboard` into the focused edit box (chat input, else a dialog edit box).
    pub fn paste(&mut self, clip: &[u8]) {
        if self.input.is_open() {
            self.input.paste(clip);
        } else if let Some(d) = self.dialogs.current_mut()
            && d.style.has_edit()
        {
            d.edit.paste(clip);
        }
    }

    /// Routes one window event; returns whether it is consumed.
    pub fn handle_event(&mut self, ev: Event, gta_menu: bool, game: &dyn GameInfo, wheel_lines: u32) -> bool {
        let r = keys::route(ev, &self.key_state(gta_menu));
        self.apply(&r.actions, game, wheel_lines);
        r.consumed
    }

    /// Enter in the chat input (ui-chat-audio.md, "Local chat input and built-in commands").
    pub fn submit(&mut self, game: &dyn GameInfo) {
        let was_open = self.input.is_open();
        let r = self.input.submit(self.netgame, self.debug_mode);
        if was_open {
            // Chat scroll position 90, clamped by the scrollbar.
            self.chat.scroll_to(SUBMIT_SCROLL_POS);
        }
        match r {
            Submit::Empty | Submit::Nothing => {}
            Submit::Chat(c) => self.outgoing.push(Outgoing::Chat(c.message)),
            Submit::Command(c) => self.outgoing.push(Outgoing::Command(c.command)),
            Submit::UnknownCommand => self.print_debug(commands::msg::UNKNOWN_COMMAND.as_bytes()),
            Submit::Local(cmd) => {
                let effects = commands::execute(&cmd, &mut self.settings, &mut self.cfg, game);
                for e in effects {
                    self.effect(e);
                }
            }
        }
    }

    /// Control-channel `say TEXT`: as if TEXT were typed into the input and Enter pressed.
    pub fn say(&mut self, text: &[u8], game: &dyn GameInfo) {
        self.input.open();
        self.input.set_text(text);
        self.submit(game);
    }

    fn effect(&mut self, e: Effect) {
        match e {
            Effect::Print(kind, text) => self.print(kind, &text),
            Effect::WriteConfig => self.write_config(),
            Effect::Quit { disconnect } => self.requests.quit = Some(disconnect),
            Effect::SendRcon(t) => self.outgoing.push(Outgoing::Rcon(t)),
            Effect::AppendFile { file, line, ok, ok_kind, failed } => {
                // `/rs` in a car writes `rawvehicles.txt` relative to the game's current directory (no
                // user-files prefix, like samp); the other files go to the user files directory.
                let path = if file == samp_ui::commands::SaveFile::RawVehicles {
                    Some(PathBuf::from(file.file_name()))
                } else {
                    self.user_dir.as_ref().map(|d| d.join(file.file_name()))
                };
                let written = path
                    .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok())
                    .is_some_and(|mut f| f.write_all(&text_mode(&line)).is_ok());
                if written {
                    self.print(ok_kind, &ok);
                } else {
                    self.print_debug(&failed);
                }
            }
            Effect::SetChatPageSize(n) => {
                self.chat.set_page_size(n);
            }
            Effect::RebuildFonts(_) => self.requests.rebuild_fonts = true,
            Effect::SetTimestamps(on) => self.chat.set_timestamps(on),
            // `/hudscalefix`: toggles the HUD aspect-ratio fix in mainhooks.rs.
            #[cfg(windows)]
            Effect::SetHudScaleFix(on) => crate::mainhooks::set_enabled(on),
            // `/testdw`: eight fixed rows through the death-message path (killer, victim, colours 0xFFFFFFFF,
            // reason); a missing killer name is empty.
            Effect::FillTestDeathWindow => {
                const ROWS: [(&[u8], &[u8], u8); 8] = [
                    (b"Pooper", b"Pooper333", 1),
                    (b"Pooper", b"Pooper", 5),
                    (b"Pooper", b"Pooper", 15),
                    (b"Pooper", b"Pooper", 14),
                    (b"Pooper", b"Pooper", 2),
                    (b"", b"PooperPooperPooper0001", 5),
                    (b"", b"Pooper", 255),
                    (b"Pooper", b"PooperPooperPooper0001", 0),
                ];
                for (k, v, reason) in ROWS {
                    self.deaths.add(k, 0xFFFF_FFFF, v, 0xFFFF_FFFF, reason);
                }
            }
            // `/dl`: the flag lives in `settings.vehicle_labels`; the labels are built by `vehlabels.rs`.
            Effect::SetVehicleLabels(_) => {}
            // Any other effect is only stored in `settings`.
            other => crate::log!("ui: effect {other:?} stored in the settings only"),
        }
    }

    /// Scoreboard row activated: RPC 23 with the row's id text (atol), then the scoreboard closes with
    /// cursor mode 0 and the delayed input restore. Mirrors samp.dll B 0x1006EBD0.
    pub fn scoreboard_activate(&mut self) {
        if !self.scoreboard.is_open() {
            return;
        }
        if let Some(row) = self.scoreboard.selected().and_then(|i| self.scoreboard.rows().get(i)) {
            self.outgoing.push(Outgoing::ClickPlayer(row.id));
        }
        self.scoreboard.close();
        self.requests.cursor.push(CursorRequest::Mode0 { immediate: false });
    }

    pub fn push_outgoing(&mut self, o: Outgoing) {
        self.outgoing.push(o);
    }

    /// Takes the queued network messages.
    pub fn take_outgoing(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outgoing)
    }
}
