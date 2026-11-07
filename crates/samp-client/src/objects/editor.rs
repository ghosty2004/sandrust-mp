//! The object editor and selection mode: RPC 27/116/117 and the per-frame editor requests.

use super::motion::start_move;
use super::*;

impl Objects {
    /// BeginObjectSelect: textdraw selection off, editor stopped (cancel 117/116), selection on.
    pub(super) fn begin_select(&mut self, net: &mut NetClient) {
        crate::ui::cancel_textdraw_selection();
        self.stop_editing(net, 0);
        self.select(true);
    }

    pub(super) fn cancel_edit(&mut self, net: &mut NetClient) {
        match self.edit {
            Edit::Object { .. } | Edit::Attached { .. } => self.stop_editing(net, 0),
            Edit::Selecting => self.select(false),
            Edit::None => {}
        }
    }

    /// BeginObjectEdit: selection and textdraw selection off, then the editor starts.
    pub(super) fn begin_edit(&mut self, e: rw::PlayerBeginObjectEdit) {
        self.select(false);
        crate::ui::cancel_textdraw_selection();
        let rotation = self.pool.get(&e.object_id).map(|o| o.rotation).unwrap_or_default();
        objedit::with(|sh| {
            sh.editor.start(objedit::Target::Object { id: e.object_id, player_object: e.player_object }, rotation)
        });
        Self::editor_input_start();
        self.edit = Edit::Object { player_object: e.player_object, object_id: e.object_id };
        crate::log!("objects: editing object {}", e.object_id);
    }

    /// BeginAttachedObjectEdit: selection off, then the editor starts in attached mode.
    pub(super) fn begin_attached_edit(&mut self, e: rw::PlayerBeginAttachedObjectEdit) {
        self.select(false);
        objedit::with(|sh| sh.editor.start(objedit::Target::Attached { slot: e.index }, [0.0; 3]));
        Self::editor_input_start();
        self.edit = Edit::Attached { index: e.index };
        crate::log!("objects: editing attached object slot {}", e.index);
    }

    /// Input side of the editor start: cursor mode 0 with an immediate restore, then the restore runs at once.
    pub(super) fn editor_input_start() {
        crate::ui::restore_input_now(true);
    }

    /// Selection on / off. Mirrors samp.dll B 0x1006DB40.
    pub(super) fn select(&mut self, on: bool) {
        let was = objedit::with(|sh| std::mem::replace(&mut sh.selection.active, on));
        if was && !on {
            crate::ui::set_input_mode(0, true);
            crate::ui::close_chat_input();
        }
        if !was && on {
            crate::ui::restore_input_now(true);
        }
        if on {
            self.edit = Edit::Selecting;
        } else if self.edit == Edit::Selecting {
            self.edit = Edit::None;
        }
    }

    /// Editor stop: cursor mode 0 (immediate), the chat input closed, then the final 117 / 116. Mirrors samp.dll
    /// B 0x10072BF0.
    pub(super) fn stop_editing(&mut self, net: &mut NetClient, response: u32) {
        if !matches!(self.edit, Edit::Object { .. } | Edit::Attached { .. }) {
            return;
        }
        objedit::with(|sh| {
            sh.editor.active = false;
            sh.editor.dragging = false;
        });
        crate::ui::set_input_mode(0, true);
        crate::ui::close_chat_input();
        self.send_edit(net, response);
        self.edit = Edit::None;
        self.edit_matrix = None;
    }

    /// Sends RPC 117 (object) or 116 (attached slot) with `response`; 117 only for an existing object, response 2
    /// at most every 250 ms (the tick of every 117 send counts).
    fn send_edit(&mut self, net: &mut NetClient, response: u32) {
        match self.edit {
            Edit::Object { player_object, object_id } => {
                let Some(o) = self.pool.get(&object_id).filter(|o| o.ptr != 0) else { return };
                let Some((_, p)) = rows(o.ptr) else { return };
                let now = tick_ms();
                if response == 2 && !objedit::update_due(now, objedit::with(|sh| sh.editor.last_send_ms)) {
                    return;
                }
                objedit::with(|sh| sh.editor.last_send_ms = now);
                let v = |a: [f32; 3]| Vec3 { x: a[0], y: a[1], z: a[2] };
                send(
                    net,
                    &rw::OnPlayerEditObject {
                        player_object,
                        object_id,
                        response,
                        position: v(p),
                        rotation: v(o.rotation),
                    },
                );
                self.sent_edit += 1;
            }
            Edit::Attached { index } => {
                let local = self.local_id.unwrap_or(u16::MAX);
                let data = self.attached.get(&(local, index)).map(|s| s.data);
                send(net, &rw::OnPlayerEditAttachedObject { response, index, data });
                self.sent_edit += 1;
            }
            _ => {}
        }
    }

    /// The editor / selection requests of this frame (window messages and the Present pass), and the per-frame
    /// data the Present pass needs (edited object, pickable objects).
    pub fn poll_editor(&mut self, runner: &mut Runner, net: &mut NetClient) {
        let actions = objedit::with(|sh| std::mem::take(&mut sh.editor.actions));
        for a in actions {
            match a {
                objedit::Action::Press => {
                    if let Edit::Object { object_id, .. } = self.edit {
                        self.edit_matrix = self.pool.get(&object_id).filter(|o| o.ptr != 0).and_then(|o| rows(o.ptr));
                    }
                }
                objedit::Action::Drag { axis, d } => self.drag(runner, net, axis, d),
                objedit::Action::Stop { response } => self.stop_editing(net, response),
                objedit::Action::Select { id } => self.send_select(net, id),
            }
        }
        let local = rd::<u32>(g::CWORLD_PLAYERS);
        let frame = match self.edit {
            Edit::Object { object_id, .. } => self
                .pool
                .get(&object_id)
                .filter(|o| o.ptr != 0)
                .map(|o| objedit::EditFrame { object: o.ptr, ped: 0, bone: 0 }),
            Edit::Attached { index } => self
                .local_id
                .and_then(|l| self.attached.get(&(l, index)))
                .filter(|s| s.ptr != 0)
                .map(|s| objedit::EditFrame { object: s.ptr, ped: local, bone: s.data.bone }),
            _ => None,
        };
        let pickable = self.pick_list();
        objedit::with(|sh| {
            sh.frame = frame;
            sh.pickable = pickable;
        });
    }

    /// Drag step. Objects: translate moves along the edit matrix with MoveObject at speed 1 (so the object
    /// follows with a lag), rotate changes the rotation copy; then a rate-limited 117 response 2. Attached
    /// objects: only the slot data changes; nothing is sent until the end. Mirrors samp.dll B 0x10073240.
    fn drag(&mut self, runner: &mut Runner, net: &mut NetClient, axis: usize, d: f32) {
        let mode = objedit::with(|sh| sh.editor.mode);
        match self.edit {
            Edit::Object { object_id, .. } => {
                let now = self.now_ms();
                let Some(o) = self.pool.get_mut(&object_id).filter(|o| o.ptr != 0) else { return };
                // Physical flag bit 3 is cleared on every step, as samp does.
                wr(o.ptr + g::PHYSICAL_FLAGS, rd::<u32>(o.ptr + g::PHYSICAL_FLAGS) & !8);
                match mode {
                    0 => {
                        let Some((r, p)) = self.edit_matrix else { return };
                        let mut v = [0.0f32; 3];
                        v[axis] = d;
                        let m = matrix16(&r, p);
                        let target = objedit::transform(&m, v);
                        self.edit_matrix = Some((r, target));
                        let keep = [world::NO_ROTATION - 1.0; 3];
                        start_move(o, target, 1.0, keep, now);
                    }
                    1 => {
                        let mut rot = o.rotation;
                        objedit::rotate(&mut rot, axis, d);
                        runner.run(
                            op::SET_OBJECT_ROTATION,
                            &[Arg::Int(o.handle), Arg::Float(rot[0]), Arg::Float(rot[1]), Arg::Float(rot[2])],
                        );
                        o.rotation = rot;
                    }
                    _ => {}
                }
                self.send_edit(net, 2);
            }
            Edit::Attached { index } => {
                let Some(l) = self.local_id else { return };
                if let Some(s) = self.attached.get_mut(&(l, index)) {
                    let (mut off, mut rot, mut sc) = (vec(s.data.offset), vec(s.data.rotation), vec(s.data.scale));
                    objedit::attached_drag(&mut off, &mut rot, &mut sc, mode, axis, d);
                    let v = |a: [f32; 3]| Vec3 { x: a[0], y: a[1], z: a[2] };
                    s.data.offset = v(off);
                    s.data.rotation = v(rot);
                    s.data.scale = v(sc);
                }
            }
            _ => {}
        }
    }

    /// RPC 27 C→S: type 0, id, model, position of an existing object. Mirrors samp.dll B 0x1006DC90.
    fn send_select(&mut self, net: &mut NetClient, id: u16) {
        let Some(o) = self.pool.get(&id).filter(|o| o.ptr != 0) else { return };
        let Some((_, p)) = rows(o.ptr) else { return };
        send(
            net,
            &rw::OnPlayerSelectObject {
                select_type: 0,
                object_id: id,
                model: o.model as u32,
                position: Vec3 { x: p[0], y: p[1], z: p[2] },
            },
        );
        self.sent_edit += 1;
    }
}

/// `GetTickCount` (samp's clock for the 117 rate).
fn tick_ms() -> u32 {
    // SAFETY: plain Win32 call.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

/// Rows + position as samp's 16-float matrix (right, forward, up, position).
fn matrix16(r: &Rotation, p: [f32; 3]) -> [f32; 16] {
    let mut m = [0.0f32; 16];
    for (i, row) in [r.right, r.forward, r.up, p].iter().enumerate() {
        m[4 * i..4 * i + 3].copy_from_slice(row);
    }
    m
}
