//! Local weapon inventory (RPCs 21/22/67/145 and spawn weapons), clip/reload
//! simulation for shooting, and the 204 delta encoder.
//!
//! Spec: docs/re/subsystems/player-state.md (22/21/67/145),
//! remote-players-sync.md §204 (+ RE-10 "204 target fields").
//! GTA weapon numbers (damage, rate, clip) are plausible values, listed in
//! docs/phase3/bot2-notes.md; they are not taken from the client.

use std::time::{Duration, Instant};

use samp_proto::sync::player::{WeaponSlot, WeaponsUpdate};
use samp_proto::sync::policy;

/// GTA weapon slot of a weapon id (`None` for ids that have no slot).
pub fn slot_of(weapon: u8) -> Option<u8> {
    Some(match weapon {
        0 | 1 => 0,
        2..=9 => 1,
        22..=24 => 2,
        25..=27 => 3,
        28 | 29 | 32 => 4,
        30 | 31 => 5,
        33 | 34 => 6,
        35..=38 => 7,
        16..=18 | 39 => 8,
        41..=43 => 9,
        10..=15 => 10,
        44..=46 => 11,
        40 => 12,
        _ => return None,
    })
}

/// Hitscan weapons: one 206 per shot (open.mp `shootable()` set).
pub fn is_hitscan(weapon: u8) -> bool {
    matches!(weapon, 22..=34 | 38)
}

/// Plausible per-weapon numbers for a scripted shooter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponInfo {
    /// GTA damage per hit; the client reports `× 0.33` in RPC 115.
    pub gta_damage: f32,
    /// Time between shots while FIRE is held.
    pub fire_interval_ms: u32,
    /// Clip size at weapon skill 1000 (dual-wield pistols/sawn-off/SMGs).
    pub clip: u16,
    pub reload_ms: u32,
    /// Hitscan range (m).
    pub range: f32,
}

pub fn info(weapon: u8) -> WeaponInfo {
    let (gta_damage, fire_interval_ms, clip, reload_ms, range) = match weapon {
        22 => (25.0, 300, 34, 1300, 35.0),
        23 => (40.0, 400, 17, 1300, 35.0),
        24 => (140.0, 800, 7, 1300, 35.0),
        25 => (10.0, 1050, 1, 0, 40.0),
        26 => (10.0, 300, 4, 1400, 35.0),
        27 => (15.0, 400, 7, 1400, 40.0),
        28 | 32 => (20.0, 75, 100, 1300, 35.0),
        29 => (25.0, 90, 30, 1300, 45.0),
        30 => (30.0, 110, 30, 1500, 70.0),
        31 => (30.0, 110, 50, 1500, 90.0),
        33 => (75.0, 1050, 1, 0, 100.0),
        34 => (125.0, 1300, 1, 0, 300.0),
        38 => (140.0, 30, 500, 0, 75.0),
        _ => (0.0, 500, 1, 0, 1.6),
    };
    WeaponInfo { gta_damage, fire_interval_ms, clip, reload_ms, range }
}

/// Damage the client reports in RPC 115 for one hit (GTA damage × 0.33).
pub fn reported_damage(weapon: u8) -> f32 {
    info(weapon).gta_damage * 0.33
}

/// The local ped's weapons: 13 slots of (weapon, total ammo) plus the armed
/// weapon and the armed weapon's clip.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub slots: [(u8, u16); 13],
    pub armed: u8,
    pub clip: u16,
    /// Reloading until this instant.
    pub reload_until: Option<Instant>,
}

impl Inventory {
    /// RPC 22 / spawn weapons: give and make current.
    pub fn give(&mut self, weapon: u8, ammo: u32) {
        let Some(slot) = slot_of(weapon) else { return };
        let ammo = ammo.min(u16::MAX.into()) as u16;
        let s = &mut self.slots[usize::from(slot)];
        *s = if s.0 == weapon { (weapon, s.1.saturating_add(ammo)) } else { (weapon, ammo) };
        self.arm(weapon);
    }

    /// RPC 21.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// RPC 67 (values outside 0..=46 are ignored by the client). Arming a
    /// weapon that is not carried is ignored here (GTA keeps the current one).
    pub fn arm(&mut self, weapon: u8) {
        if weapon > 46 {
            return;
        }
        if weapon != 0 && !self.carries(weapon) {
            return;
        }
        self.armed = weapon;
        self.clip = self.ammo(weapon).min(info(weapon).clip);
        self.reload_until = None;
    }

    /// RPC 145: only if carried.
    pub fn set_ammo(&mut self, weapon: u8, ammo: u16) {
        if let Some(slot) = slot_of(weapon)
            && self.slots[usize::from(slot)].0 == weapon
        {
            self.slots[usize::from(slot)].1 = ammo;
            if self.armed == weapon {
                self.clip = self.clip.min(ammo);
            }
        }
    }

    pub fn carries(&self, weapon: u8) -> bool {
        slot_of(weapon).is_some_and(|s| self.slots[usize::from(s)].0 == weapon && weapon != 0)
    }

    pub fn ammo(&self, weapon: u8) -> u16 {
        slot_of(weapon).map_or(0, |s| {
            let (w, a) = self.slots[usize::from(s)];
            if w == weapon { a } else { 0 }
        })
    }

    pub fn reloading(&self, now: Instant) -> bool {
        self.reload_until.is_some_and(|t| now < t)
    }

    /// Finish a reload whose time is up.
    pub fn update(&mut self, now: Instant) {
        if self.reload_until.is_some_and(|t| now >= t) {
            self.reload_until = None;
            self.clip = self.ammo(self.armed).min(info(self.armed).clip);
        }
    }

    /// Fire one round of the armed weapon if possible. Starts a reload when
    /// the clip runs dry and ammo is left.
    pub fn fire(&mut self, now: Instant) -> bool {
        self.update(now);
        let w = self.armed;
        if self.reloading(now) || self.clip == 0 {
            return false;
        }
        let Some(slot) = slot_of(w) else { return false };
        let s = &mut self.slots[usize::from(slot)];
        s.1 = s.1.saturating_sub(1);
        self.clip -= 1;
        if self.clip == 0 && s.1 > 0 {
            let info = info(w);
            if info.reload_ms == 0 {
                self.clip = s.1.min(info.clip);
            } else {
                self.reload_until = Some(now + Duration::from_millis(info.reload_ms.into()));
            }
        }
        true
    }

    /// Aim-sync weapon state: 3 while reloading, else `min(ammoInClip, 2)`.
    pub fn weapon_state(&self, now: Instant) -> u8 {
        if self.reloading(now) { 3 } else { self.clip.min(2) as u8 }
    }
}

/// 204 delta encoder: last-sent slot copy, targets and armed weapon.
#[derive(Debug)]
pub struct WeaponsTracker {
    sent: [(u8, u16); 13],
    targets: (u16, u16),
    armed: u8,
    last_send: Option<Instant>,
}

impl Default for WeaponsTracker {
    fn default() -> Self {
        Self { sent: [(0, 0); 13], targets: (0xffff, 0xffff), armed: 0, last_send: None }
    }
}

impl WeaponsTracker {
    /// Evaluate once per tick. Returns the packet to send, if any.
    ///
    /// Unchanged targets and armed weapon within 100 ms of the last send →
    /// nothing. Otherwise changed slots are appended (copy updated); sent only
    /// if the targets changed or a slot record was appended.
    pub fn evaluate(
        &mut self,
        inv: &Inventory,
        target_player: u16,
        target_actor: u16,
        now: Instant,
    ) -> Option<WeaponsUpdate> {
        let targets = (target_player, target_actor);
        let throttle = Duration::from_millis(policy::WEAPONS_MIN_INTERVAL_MS.into());
        if targets == self.targets
            && inv.armed == self.armed
            && self.last_send.is_some_and(|t| now.duration_since(t) < throttle)
        {
            return None;
        }
        self.armed = inv.armed;
        let mut slots = Vec::new();
        for (i, (cur, sent)) in inv.slots.iter().zip(self.sent.iter_mut()).enumerate() {
            if cur != sent {
                *sent = *cur;
                slots.push(WeaponSlot { slot: i as u8, weapon_id: cur.0, ammo: cur.1 });
            }
        }
        let targets_changed = targets != self.targets;
        self.targets = targets;
        if !targets_changed && slots.is_empty() {
            return None;
        }
        self.last_send = Some(now);
        Some(WeaponsUpdate { target_player, target_actor, slots })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_follow_gta_table() {
        assert_eq!(slot_of(24), Some(2));
        assert_eq!(slot_of(31), Some(5));
        assert_eq!(slot_of(4), Some(1));
        assert_eq!(slot_of(46), Some(11));
        assert_eq!(slot_of(19), None);
    }

    #[test]
    fn give_fire_reload() {
        let t0 = Instant::now();
        let mut inv = Inventory::default();
        inv.give(24, 8);
        assert_eq!((inv.armed, inv.clip, inv.ammo(24)), (24, 7, 8));
        for _ in 0..7 {
            assert!(inv.fire(t0));
        }
        assert_eq!(inv.weapon_state(t0), 3);
        assert!(!inv.fire(t0));
        let later = t0 + Duration::from_millis(1300);
        assert!(inv.fire(later));
        assert_eq!(inv.ammo(24), 0);
        assert!(!inv.fire(later));
    }

    #[test]
    fn weapons_update_delta_and_throttle() {
        let t0 = Instant::now();
        let mut inv = Inventory::default();
        let mut tr = WeaponsTracker::default();
        assert_eq!(tr.evaluate(&inv, 0xffff, 0xffff, t0), None);
        inv.give(24, 50);
        let p = tr.evaluate(&inv, 0xffff, 0xffff, t0).expect("slot 2 changed");
        assert_eq!(p.slots, vec![WeaponSlot { slot: 2, weapon_id: 24, ammo: 50 }]);
        inv.fire(t0);
        // Same armed weapon and targets within 100 ms: throttled.
        assert_eq!(tr.evaluate(&inv, 0xffff, 0xffff, t0 + Duration::from_millis(50)), None);
        let p = tr.evaluate(&inv, 0xffff, 0xffff, t0 + Duration::from_millis(100)).expect("ammo changed");
        assert_eq!(p.slots[0].ammo, 49);
        // A target change is sent even without slot changes, and bypasses the throttle.
        let p = tr.evaluate(&inv, 3, 0xffff, t0 + Duration::from_millis(110)).expect("target changed");
        assert!(p.slots.is_empty());
        assert_eq!(p.target_player, 3);
    }
}
