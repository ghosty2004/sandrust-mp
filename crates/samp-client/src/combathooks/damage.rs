//! Damage hooks: ComputeDamageResponse, CEventDamage::AffectsPed and CVehicle::InflictDamage.

use super::*;

// ---- H6: ComputeDamageResponse -----------------------------------------------------------------------------

/// Decision for `CPedDamageResponseCalculator` `calc` and `victim`. Returns 1 = cancel. Mirrors samp.dll 0x100A3BF0.
extern "C" fn damage_decide(calc: u32, victim: u32) -> u32 {
    if calc == 0 || victim == 0 {
        return 0;
    }
    N_DAMAGE.fetch_add(1, Ordering::Relaxed);
    let local = local_ped();
    let damager: u32 = rd(calc);
    let damage: f32 = rd(calc + 4);
    let body_part: u32 = rd(calc + 8);
    let weapon: u32 = rd(calc + 0xC);
    let decision = with_shared(|sh| {
        let victim_local = victim == local;
        let damager_local = damager == local && local != 0;
        let ctx = DamageCtx {
            victim_local,
            damager_local,
            victim_civilian: rd::<u32>(victim) == g::VTBL_CIVILIAN_PED,
            surface_is_damager: combat::surf_inputs(sh.surface, damager).1,
            surfing: combat::surf_inputs(sh.surface, damager).0,
            weapon,
            attacker: if victim_local { sh.player_of_entity(damager).map(Shared::rref) } else { None },
            victim_remote: if damager_local { sh.player_of_entity(victim).map(Shared::rref) } else { None },
            victim_actor: if damager_local { sh.actors.iter().find(|a| a.0 == victim).map(|a| a.1) } else { None },
            local_team: sh.local_team,
        };
        combat::decide(&ctx)
    });
    if N_DAMAGE.load(Ordering::Relaxed) <= LOG_LIMIT {
        crate::log!(
            "combat: damage response victim {victim:#x} (local {}) damager {damager:#x} weapon {weapon} damage {damage} part {body_part} -> {decision:?}",
            victim == local
        );
    }
    let Some(d) = decision else { return 0 };
    match d.report {
        Some(Report::Take { attacker }) => queue(Outgoing::Damage {
            taking: true,
            player: attacker,
            damage: damage * combat::DAMAGE_SCALE,
            weapon,
            body_part,
        }),
        Some(Report::Give { victim }) => queue(Outgoing::Damage {
            taking: false,
            player: victim,
            damage: damage * combat::DAMAGE_SCALE,
            weapon,
            body_part,
        }),
        Some(Report::Actor { actor }) => queue(Outgoing::ActorDamage { actor, damage, weapon, body_part }),
        None => {}
    }
    u32::from(d.cancel)
}

/// Entry stub of `ComputeDamageResponse(CPed* victim, CPedDamageResponse&, bool)` (thiscall, 3 arguments).
#[unsafe(naked)]
pub(super) unsafe extern "C" fn damage_stub() {
    std::arch::naked_asm!(
        "test ecx, ecx",
        "jz 2f",
        "push ecx",
        "push edx",
        "push dword ptr [esp + 12]",
        "push ecx",
        "call {decide}",
        "add esp, 8",
        "pop edx",
        "pop ecx",
        "test eax, eax",
        "jz 2f",
        "ret 12",
        "2:",
        "mov eax, dword ptr fs:[0]",
        "push {resume}",
        "ret",
        decide = sym damage_decide,
        resume = const site::COMPUTE_DAMAGE_RESPONSE_RESUME,
    )
}

// ---- H7: CEventDamage::AffectsPed ---------------------------------------------------------------------------

/// Entry stub of `CEventDamage::AffectsPed(CPed*)` (thiscall): true at once for the local ped while a remote
/// context is active. Mirrors samp.dll 0x100A53E0.
#[unsafe(naked)]
pub(super) unsafe extern "C" fn affects_ped_stub() {
    std::arch::naked_asm!(
        "push eax",
        "mov eax, dword ptr [{players}]",
        "test eax, eax",
        "jz 2f",
        "cmp eax, dword ptr [esp + 8]",
        "jne 2f",
        "cmp byte ptr [{focus}], 0",
        "je 2f",
        "pop eax",
        "mov eax, 1",
        "ret 4",
        "2:",
        "pop eax",
        "sub esp, 12",
        "push esi",
        "mov esi, ecx",
        "push {resume}",
        "ret",
        players = const g::CWORLD_PLAYERS,
        focus = const sw::PLAYER_IN_FOCUS,
        resume = const site::AFFECTS_PED_RESUME,
    )
}

// ---- H8: CVehicle::InflictDamage ---------------------------------------------------------------------------

/// Cancel (the stub returns with `ret 0x18`) when the vehicle is the one the local player drives and the damager
/// maps to a player of the local team. Mirrors samp.dll 0x100A5280.
extern "C" fn inflict_decide(vehicle: u32, damager: u32) -> u32 {
    let local = local_ped();
    if local == 0 || vehicle == 0 || rd::<u32>(vehicle + g::VEH_DRIVER) != local {
        return 0;
    }
    with_shared(|sh| {
        let team = sh.local_team;
        let r = sh.player_of_entity(damager);
        u32::from(team != 255 && r.is_some_and(|r| r.team == team))
    })
    .unwrap_or(0)
}

/// Entry stub of `CVehicle::InflictDamage(CEntity*, eWeaponType, float, CVector)` (thiscall, 6 dwords).
#[unsafe(naked)]
pub(super) unsafe extern "C" fn inflict_stub() {
    std::arch::naked_asm!(
        "push ecx",
        "push edx",
        "push dword ptr [esp + 12]",
        "push ecx",
        "call {decide}",
        "add esp, 8",
        "pop edx",
        "pop ecx",
        "test eax, eax",
        "jz 2f",
        "xor eax, eax",
        "ret 0x18",
        "2:",
        "push -1",
        "push {resume}",
        "ret",
        decide = sym inflict_decide,
        resume = const site::INFLICT_DAMAGE_CONTINUE,
    )
}
