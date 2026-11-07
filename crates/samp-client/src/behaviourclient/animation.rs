//! Animations: the AddAnimation / BlendAnimation recorders, the local animation encoder, RPC 86/87 and the
//! remote animation state.

use super::*;

// ---- the AddAnimation / BlendAnimation recorders ------------------------------------------------------------------

/// Last recorded animation call on the local clump: group, id, blend delta. (samp also records whether it was an
/// add or a blend; its encoder never reads that, so we do not keep it.)
static REC_GROUP: AtomicU32 = AtomicU32::new(0);
static REC_ID: AtomicU32 = AtomicU32::new(0);
static REC_DELTA: AtomicU32 = AtomicU32::new(0);

fn record(clump: u32, group: u32, id: u32, delta: Option<u32>) {
    let ped = local_ped();
    let local_clump: u32 = if ped != 0 { rd(ped + ENTITY_RW_OBJECT) } else { 0 };
    if clump != local_clump || (group == 0 && id == 0xA0) {
        return;
    }
    REC_GROUP.store(group, Ordering::Relaxed);
    REC_ID.store(id, Ordering::Relaxed);
    if let Some(d) = delta {
        REC_DELTA.store(d, Ordering::Relaxed);
    }
}

extern "C" fn record_add(clump: u32, group: u32, id: u32) {
    record(clump, group, id, None);
}

extern "C" fn record_blend(clump: u32, group: u32, id: u32, delta: u32) {
    record(clump, group, id, Some(delta));
}

/// `CAnimManager::AddAnimation(clump, group, id)` entry: record, then the two displaced instructions and the rest.
#[unsafe(naked)]
unsafe extern "C" fn add_animation_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x2C]",
        "push dword ptr [esp+0x2C]",
        "push dword ptr [esp+0x2C]",
        "call {rec}",
        "add esp, 12",
        "popad",
        "mov eax, [esp+0xC]",
        "mov edx, dword ptr [0xB4EA34]",
        "push {cont}",
        "ret",
        rec = sym record_add,
        cont = const 0x4D3AAAu32,
    )
}

/// `CAnimManager::BlendAnimation(clump, group, id, delta)` entry.
#[unsafe(naked)]
unsafe extern "C" fn blend_animation_stub() {
    core::arch::naked_asm!(
        "pushad",
        "push dword ptr [esp+0x30]",
        "push dword ptr [esp+0x30]",
        "push dword ptr [esp+0x30]",
        "push dword ptr [esp+0x30]",
        "call {rec}",
        "add esp, 16",
        "popad",
        "sub esp, 0x14",
        "mov ecx, [esp+0x18]",
        "push {cont}",
        "ret",
        rec = sym record_blend,
        cont = const 0x4D4617u32,
    )
}

/// `CAnimManager::UncompressAnimation(hier)` guard: returns at once for a NULL hierarchy, otherwise runs the two
/// displaced instructions and continues in the original. Mirrors samp.dll 0x100A3E00.
#[unsafe(naked)]
unsafe extern "C" fn uncompress_animation_stub() {
    core::arch::naked_asm!(
        "push edi",
        "mov edi, [esp+0x8]",
        "test edi, edi",
        "jz 2f",
        "push {cont}",
        "ret",
        "2:",
        "pop edi",
        "ret",
        cont = const 0x4D41C5u32,
    )
}

/// Installs this module's patches; each result is tagged with its row in docs/re/data/gta-patches-r4.csv. The
/// recorders are `jmp dword [slot]` at the function entries, with the handler addresses in the NOP padding before them.
pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Vec<(u32, Result<(), PatchError>)> {
    let mut out = Vec::new();
    // POINT_CAMERA_AT_CHAR always takes the non-player branch, so spectating a remote CPlayerPed works.
    out.push((198, patcher.write(mem, "POINT_CAMERA_AT_CHAR ped mode", 0x47C477, &[0xEB], Some(&[0x74]))));
    type Row = (u32, u32, u32, &'static [u8], u32, &'static str);
    let rows: [Row; 2] = [
        (
            17,
            0x4D3AA0,
            0x4D3934,
            &[0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x15, 0x34, 0xEA, 0xB4, 0x00],
            add_animation_stub as unsafe extern "C" fn() as usize as u32,
            "CAnimManager::AddAnimation recorder",
        ),
        (
            18,
            0x4D4610,
            0x4D4609,
            &[0x83, 0xEC, 0x14, 0x8B, 0x4C, 0x24, 0x18],
            blend_animation_stub as unsafe extern "C" fn() as usize as u32,
            "CAnimManager::BlendAnimation recorder",
        ),
    ];
    let guard = uncompress_animation_stub as unsafe extern "C" fn() as usize as u32;
    out.push((
        54,
        patcher.hook_jmp(
            mem,
            "CAnimManager::UncompressAnimation NULL guard",
            0x4D41C0,
            guard,
            Some(&[0x57, 0x8B, 0x7C, 0x24, 0x08]),
        ),
    ));
    for (row, site, slot, orig, handler, label) in rows {
        let r = patcher.write(mem, label, slot, &handler.to_le_bytes(), Some(&[0x90; 4])).and_then(|()| {
            let mut jmp = vec![0xFF, 0x25];
            jmp.extend_from_slice(&slot.to_le_bytes());
            jmp.resize(orig.len(), 0x90);
            patcher.write(mem, label, site, &jmp, Some(orig))
        });
        out.push((row, r));
    }
    out
}

/// Applies the animation field of one stored 207 of a remote player; called from the one-shot apply in players.rs.
pub fn remote_anim(runner: &mut Runner, id: u16, ped: u32, anim: u32) {
    let actions = remote_states(|m| m.entry(id).or_default().step(anim, || find_task(ped, &[TASK_NAMED_ANIM]) != 0));
    for a in actions {
        match a {
            RemoteAction::Play(d) => {
                let Some((lib, name)) = anims::record(i32::from(d.index)).and_then(anims::split) else { continue };
                play(
                    runner,
                    ped,
                    name.as_bytes(),
                    lib.as_bytes(),
                    d.delta,
                    [d.looping, d.lock_x, d.lock_y, d.freeze],
                    d.time,
                );
                // The parachute object follows the ped's PARACHUTE animations.
                if lib == "PARACHUTE" {
                    crate::specials::remote_para_anim(runner, id, ped, name);
                }
            }
            RemoteAction::VendEat => play(runner, ped, b"VEND_EAT_P", b"VENDING", 4.0, [false; 4], -1),
            RemoteAction::Stop => play(runner, ped, b"crry_prtial", b"CARRY", 4.0, [false; 4], 0),
        }
    }
}

/// Remote animation state per player; shared with the one-shot apply in players.rs.
static REMOTE_ANIMS: std::sync::Mutex<Option<HashMap<u16, RemoteState>>> = std::sync::Mutex::new(None);

fn remote_states<R>(f: impl FnOnce(&mut HashMap<u16, RemoteState>) -> R) -> R {
    let mut g = REMOTE_ANIMS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}
/// samp's animation player: "SEX" is refused; an unloaded library is requested and polled with `Sleep(1)` 15 times,
/// then given up; otherwise `TASK_PLAY_ANIM_NON_INTERRUPTABLE`. Mirrors samp.dll B 0x100AD240.
pub(crate) fn play(runner: &mut Runner, ped: u32, name: &[u8], lib: &[u8], delta: f32, flags: [bool; 4], time: i32) {
    let Some(h) = game::ped_handle(&ProcessMemory, ped).ok().flatten() else { return };
    if anims::refused_library(lib) {
        return;
    }
    if !runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Bytes(lib)]) {
        runner.run(op::REQUEST_ANIMATION, &[Arg::Bytes(lib)]);
        let mut loaded = runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Bytes(lib)]);
        let mut polls = 0;
        while !loaded {
            std::thread::sleep(Duration::from_millis(1));
            polls += 1;
            if polls == ANIM_LOAD_POLLS {
                return;
            }
            loaded = runner.run(op::HAS_ANIMATION_LOADED, &[Arg::Bytes(lib)]);
        }
    }
    let f = |b: bool| Arg::Int(i32::from(b));
    runner.run(
        op::TASK_PLAY_ANIM_NON_INTERRUPTABLE,
        &[
            Arg::Int(h),
            Arg::Bytes(name),
            Arg::Bytes(lib),
            Arg::Float(delta),
            f(flags[0]),
            f(flags[1]),
            f(flags[2]),
            f(flags[3]),
            Arg::Int(time),
        ],
    );
}

impl Behaviour {
    // ---- animations ------------------------------------------------------------------------------------------

    /// Every alive frame: the local animation encoder feeds the 207 latch.
    pub fn anim_tick(&mut self, ped: u32) {
        let v = self.encode_local(ped);
        self.anim.update(v);
    }

    /// The local animation encoder. Mirrors samp.dll B 0x100AE7A0.
    fn encode_local(&mut self, ped: u32) -> u32 {
        if !entity_live(ped) || rd::<u32>(ped + ENTITY_RW_OBJECT) == 0 {
            return 0;
        }
        let task = find_task(ped, &[TASK_NAMED_ANIM, TASK_NAMED_ANIM_2]);
        if task != 0 {
            let lib = c_str_at(task + TASK_ANIM_GROUP, 16);
            let name = c_str_at(task + TASK_ANIM_NAME, 24);
            let index = anims::index_of(&lib, &name);
            return anims::encode_task(
                index,
                rd(task + TASK_ANIM_DELTA),
                rd(task + TASK_ANIM_FLAGS),
                rd(task + TASK_ANIM_TIME),
            );
        }
        self.hash_table();
        let (group, id) = (REC_GROUP.load(Ordering::Relaxed), REC_ID.load(Ordering::Relaxed));
        // SAFETY: GTA's cdecl static-association lookup, main thread.
        let assoc = unsafe {
            let f: extern "C" fn(u32, u32) -> u32 = std::mem::transmute(GET_ANIM_ASSOCIATION as usize);
            f(group, id)
        };
        let hash = if assoc != 0 { rd::<u32>(rd::<u32>(assoc + 0x10)) } else { 0 };
        let index = anims::index_of_hash(&self.hashes, hash);
        anims::encode_plain(index, f32::from_bits(REC_DELTA.load(Ordering::Relaxed)))
    }

    /// RPC 86: the local ped, or a streamed-in remote ped (marked as animated by the server).
    /// Mirrors samp.dll B 0x1001A900.
    pub fn apply_animation(
        &mut self,
        runner: &mut Runner,
        players: &crate::players::Players,
        local_id: Option<u16>,
        a: &ApplyPlayerAnimation,
    ) {
        let ped = if Some(a.player_id) == local_id {
            local_ped()
        } else {
            if a.player_id >= 1004 {
                return;
            }
            let Some(p) = players.get(a.player_id) else { return };
            remote_states(|m| m.entry(a.player_id).or_default().by_rpc = true);
            p.ped
        };
        if ped == 0 {
            return;
        }
        play(runner, ped, &a.name, &a.lib, a.delta, [a.looping, a.lock_x, a.lock_y, a.freeze], a.time as i32);
    }

    /// RPC 87: the ped is re-set to its own position (which flushes its tasks); a remote loses the RPC mark.
    /// Mirrors samp.dll B 0x10018530.
    pub fn clear_animations(&mut self, players: &crate::players::Players, local_id: Option<u16>, player_id: u16) {
        let ped = if Some(player_id) == local_id {
            local_ped()
        } else {
            if player_id >= 1004 {
                return;
            }
            let Some(p) = players.get(player_id) else { return };
            remote_states(|m| m.entry(player_id).or_default().by_rpc = false);
            p.ped
        };
        if ped != 0
            && let Some(p) = matrix_pos(ped)
        {
            teleport_entity(ped, p);
        }
    }

    /// A remote player streamed in or out: its animation state starts over, as in samp's remote constructor.
    pub fn forget_remote(&mut self, id: u16) {
        remote_states(|m| m.remove(&id));
    }
}
