//! Weapon skills while rendering and the smaller weapon hooks (crosshairs, camera shake, photographs,
//! satchels, detonators, fire).

use super::*;

// ---- H9 and small hooks ------------------------------------------------------------------------------------

/// Replaces `call CPed::GetWeaponSkill` in `RenderWeaponPedsForPC`: a remote ped's skill comes from its slot's
/// skill table. Mirrors samp.dll 0x100A2D80.
pub(super) extern "thiscall" fn render_weapon_skill(ped: u32) -> u32 {
    // SAFETY: GTA's own function with the same signature.
    let f: extern "thiscall" fn(u32) -> u32 = unsafe { std::mem::transmute(g::GET_WEAPON_SKILL as usize) };
    let skills = crate::swap::with_table(|t| t.slot_of(ped).map(|s| t.shadows[s].skills));
    match skills {
        Some(sk) => {
            let saved: [f32; SKILLS] = read_floats(sw::WEAPON_SKILLS);
            write_floats(sw::WEAPON_SKILLS, &sk);
            let r = f(ped);
            write_floats(sw::WEAPON_SKILLS, &saved);
            r
        }
        None => f(ped),
    }
}

/// Replaces `call CamShakeNoPos` in `FireSniper` with a bare `ret`, like samp.
pub(super) extern "C" fn no_cam_shake() {}

/// Crosshair wrapper: samp's HUD scale fix around the draw while the local camera mode is 0x35 (`mainhooks.rs`).
/// Mirrors samp.dll 0x100A2590.
pub(super) extern "C" fn crosshairs_site() {
    crate::mainhooks::crosshairs(|| {
        // SAFETY: the original cdecl function.
        unsafe {
            let f: extern "C" fn() = std::mem::transmute(g::DRAW_CROSSHAIRS as usize);
            f();
        }
    });
}

/// `CWeapon::TakePhotograph`: the photo flags only for the local photographer. Mirrors samp.dll 0x100A3DB0.
/// Entered with the photographer at [esp+8] (the jmp replaces an instruction inside the function body).
#[unsafe(naked)]
pub(super) unsafe extern "C" fn take_photo_stub() {
    std::arch::naked_asm!(
        "push eax",
        "mov eax, dword ptr [esp + 12]",
        "cmp eax, dword ptr [{players}]",
        "jne 2f",
        "mov byte ptr [{f1}], 1",
        "mov byte ptr [{f2}], 1",
        "2:",
        "pop eax",
        "push {resume}",
        "ret",
        players = const g::CWORLD_PLAYERS,
        f1 = const site::PHOTO_TAKEN,
        f2 = const site::PHOTO_TAKEN_2,
        resume = const site::TAKE_PHOTO_RESUME,
    )
}

/// `CProjectileInfo::Update` at the type-39 (satchel) test: ESI = the projectile object, EBX = the projectile info;
/// an owner-less satchel is removed, then the loop continues with DL = 0xFF. Mirrors samp.dll 0x100A3FC0.
#[unsafe(naked)]
pub(super) unsafe extern "C" fn satchel_update_stub() {
    std::arch::naked_asm!(
        "cmp dword ptr [ebx + 4], 0",
        "jne 2f",
        "pushad",
        "push esi",
        "mov eax, {remove}",
        "call eax",
        "add esp, 4",
        "popad",
        "mov dl, 0xFF",
        "jmp 3f",
        "2:",
        "mov ecx, dword ptr [ebx + 4]",
        "mov dl, byte ptr [ecx + 0x36]",
        "3:",
        "push {resume}",
        "ret",
        remove = const g::REMOVE_IF_PROJECTILE,
        resume = const site::SATCHEL_UPDATE_RESUME,
    )
}

/// `CProjectileInfo::RemoveDetonatorProjectiles` at the type-39 test: only satchels owned by the detonating ped
/// ([esp]) are blown. Mirrors samp.dll 0x100A4000.
#[unsafe(naked)]
pub(super) unsafe extern "C" fn detonator_stub() {
    std::arch::naked_asm!(
        "cmp dword ptr [esi - 0x20], 0x27",
        "mov edi, dword ptr [ebp]",
        "jne 2f",
        "mov eax, dword ptr [esp]",
        "cmp dword ptr [esi - 0x1C], eax",
        "jne 2f",
        "push {blow}",
        "ret",
        "2:",
        "push {skip}",
        "ret",
        blow = const site::DETONATOR_BLOW,
        skip = const site::DETONATOR_SKIP,
    )
}

/// Constants samp points GTA at (the aim time step and the bullet bounds).
pub(super) static TWO: f32 = 2.0;
pub(super) static BOUND_MIN: f32 = -20000.0;
pub(super) static BOUND_MAX: f32 = 20000.0;

/// Extinguishes the ped's fire and its vehicle's. Mirrors samp.dll 0x100AD130.
pub fn extinguish(ped: u32) {
    let fire: u32 = rd(ped + g::PED_FIRE);
    // SAFETY: GTA's CFire::Extinguish (thiscall).
    let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(g::FIRE_EXTINGUISH as usize) };
    if fire != 0 {
        f(fire);
    }
    let veh: u32 = rd(ped + g::PED_VEHICLE);
    if rd::<u32>(ped + g::PED_FLAGS) & (1 << 8) != 0 && veh != 0 {
        let vf: u32 = rd(veh + g::VEH_FIRE);
        if vf != 0 {
            f(vf);
        }
    }
}
