//! `/dl` vehicle debug labels, and the InitGame vehicle-model preload they report.
//!
//! * Labels: while `/dl` is on, every vehicle within 20 m of the local player (of the camera while spectating)
//!   gets a 3D text label with its id, model, subtype, health, preload flag, distance, passenger seats and its
//!   current and stream-in positions; no line-of-sight test. `tick` builds them, the UI pass draws `draws`.
//! * Preload (`preload`): on InitGame, loaded vehicle models are released, then the up to 80 most used models
//!   (InitGame's per-model usage counts) are requested and flagged "preloaded".
//!
//! Spec: docs/re/subsystems/ui-rendering.md, "2.3 Present: per-frame draw order".

/// Vehicle models covered by the InitGame usage table.
pub const FIRST_MODEL: u16 = 400;
pub const MODEL_COUNT: usize = 212;
/// Entries of the sorted table that are preloaded.
pub const PRELOAD_SLOTS: usize = 80;
/// Label colour and range in metres.
pub const LABEL_ARGB: u32 = 0xFF35_8BD4;
pub const LABEL_RANGE: f32 = 20.0;

/// The models samp preloads for these usage counts, in request order: sorted by count, highest first, model
/// order kept among equal counts; at most [`PRELOAD_SLOTS`], zero counts skipped.
pub fn preload_models(counts: &[u8]) -> Vec<u16> {
    let mut v: Vec<(u8, u16)> =
        (0..MODEL_COUNT).map(|i| (counts.get(i).copied().unwrap_or(0), FIRST_MODEL + i as u16)).collect();
    // Stable sort by count, descending (samp's bubble sort only swaps on a strictly smaller count).
    v.sort_by_key(|e| std::cmp::Reverse(e.0));
    v.iter().take(PRELOAD_SLOTS).filter(|(c, _)| *c != 0).map(|&(_, m)| m).collect()
}

/// Vehicle subtype from the GTA vtable (0 for an unknown vtable). Mirrors samp.dll 0x100B6220.
pub fn subtype(vtable: u32) -> u8 {
    match vtable {
        0x0087_1120 => 1,
        0x0087_21A0 => 4,
        0x0087_1360 => 2,
        0x0087_1948 => 5,
        0x0087_1680 => 3,
        0x0087_1528 => 6,
        0x0087_2370 => 7,
        _ => 0,
    }
}

/// One label's values.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: u16,
    pub model: u16,
    pub subtype: u8,
    pub health: f32,
    pub preloaded: bool,
    pub distance: f32,
    pub seats: u8,
    pub pos: [f32; 3],
    pub spawn: [f32; 3],
}

/// The label text, in samp's format.
pub fn text(r: &Row) -> String {
    format!(
        "[id: {}, type: {} subtype: {} Health: {:.1} preloaded: {}]\nDistance: {:.2}m\nPassengerSeats: {}\n\
         cPos: {:.3},{:.3},{:.3}\nsPos: {:.3},{:.3},{:.3}",
        r.id,
        r.model,
        r.subtype,
        r.health,
        u8::from(r.preloaded),
        r.distance,
        r.seats,
        r.pos[0],
        r.pos[1],
        r.pos[2],
        r.spawn[0],
        r.spawn[1],
        r.spawn[2]
    )
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::game::off;
    use crate::patch::{Memory, ProcessMemory};
    use crate::script::{Arg, Runner, op};
    use std::sync::Mutex;

    /// Preload flag per model, indexed from [`FIRST_MODEL`].
    static PRELOADED: Mutex<[bool; MODEL_COUNT]> = Mutex::new([false; MODEL_COUNT]);
    /// One label: world position and text.
    pub type Draw = ([f32; 3], String);
    /// The labels of the current frame (`None` while `/dl` is off), read by the UI pass.
    static DRAWS: Mutex<Option<Vec<Draw>>> = Mutex::new(None);

    /// InitGame: releases every loaded vehicle model, then requests and flags the preload set.
    /// Mirrors samp.dll 0x100A1620 (build B).
    pub fn preload(runner: &mut Runner, counts: &[u8]) {
        let mut flags = PRELOADED.lock().unwrap_or_else(|e| e.into_inner());
        *flags = [false; MODEL_COUNT];
        for i in 0..MODEL_COUNT as i32 {
            let m = i32::from(FIRST_MODEL) + i;
            if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(m)]) {
                runner.run(op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(m)]);
            }
        }
        let models = preload_models(counts);
        for &m in &models {
            runner.run(op::REQUEST_MODEL, &[Arg::Int(i32::from(m))]);
            runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
            flags[usize::from(m - FIRST_MODEL)] = true;
        }
        crate::log!("vehlabels: InitGame preload of {} vehicle models: {models:?}", models.len());
    }

    fn v3(a: u32) -> [f32; 3] {
        let m = ProcessMemory;
        [m.read_f32(a).unwrap_or(0.0), m.read_f32(a + 4).unwrap_or(0.0), m.read_f32(a + 8).unwrap_or(0.0)]
    }

    fn matrix_pos(e: u32) -> Option<[f32; 3]> {
        let m = ProcessMemory.read_u32(e + off::PLACEABLE_MATRIX).ok().filter(|&m| m != 0)?;
        Some(v3(m + off::MATRIX_POS))
    }

    /// Client tick: rebuild the labels while `/dl` is on. `spectating` selects the camera as the reference point.
    /// `rows` holds (vehicle id, `CVehicle*`, stream-in position) per streamed-in vehicle.
    pub fn tick(rows: Vec<(u16, u32, [f32; 3])>, local_ped: u32, spectating: bool) {
        const CAMERA_MATRIX_POS: u32 = 0xB6F9CC;
        if !crate::ui::vehicle_labels_on() {
            *DRAWS.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }
        let from = if spectating { Some(v3(CAMERA_MATRIX_POS)) } else { matrix_pos(local_ped) };
        let flags = *PRELOADED.lock().unwrap_or_else(|e| e.into_inner());
        let m = ProcessMemory;
        let mut out = Vec::new();
        for (id, ptr, spawn) in rows {
            let (Some(pos), Some(from)) = (matrix_pos(ptr), from) else { continue };
            let d = ((pos[0] - from[0]).powi(2) + (pos[1] - from[1]).powi(2) + (pos[2] - from[2]).powi(2)).sqrt();
            if d > LABEL_RANGE {
                continue;
            }
            let model = m.read_u16(ptr + off::ENTITY_MODEL).unwrap_or(0);
            let row = Row {
                id,
                model,
                subtype: subtype(m.read_u32(ptr).unwrap_or(0)),
                health: m.read_f32(ptr + 0x4C0).unwrap_or(0.0), // CVehicle::m_fHealth
                preloaded: model
                    .checked_sub(FIRST_MODEL)
                    .and_then(|i| flags.get(usize::from(i)).copied())
                    .unwrap_or(false),
                distance: d,
                seats: m.read_u8(ptr + 0x488).unwrap_or(0), // CVehicle::m_nMaxPassengers
                pos,
                spawn,
            };
            out.push((pos, text(&row)));
        }
        *DRAWS.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
    }

    /// The labels for the UI pass (`None` = `/dl` off: no batch at all).
    pub fn draws() -> Option<Vec<Draw>> {
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preload_order_is_stable_and_capped() {
        let mut c = vec![0u8; MODEL_COUNT];
        c[11] = 3; // 411
        c[0] = 5; // 400
        c[160] = 3; // 560
        c[2] = 1; // 402
        assert_eq!(preload_models(&c), vec![400, 411, 560, 402]);
        let all = vec![1u8; MODEL_COUNT];
        let p = preload_models(&all);
        assert_eq!((p.len(), p[0], p[79]), (80, 400, 479));
    }

    #[test]
    fn label_text() {
        let r = Row {
            id: 3,
            model: 560,
            subtype: 1,
            health: 1000.0,
            preloaded: false,
            distance: 6.123,
            seats: 3,
            pos: [1706.0, -2600.0, 13.5],
            spawn: [1706.0, -2600.0, 13.55],
        };
        assert_eq!(
            text(&r),
            "[id: 3, type: 560 subtype: 1 Health: 1000.0 preloaded: 0]\nDistance: 6.12m\nPassengerSeats: 3\n\
             cPos: 1706.000,-2600.000,13.500\nsPos: 1706.000,-2600.000,13.550"
        );
        assert_eq!(subtype(0x871120), 1);
        assert_eq!(subtype(0x872370), 7);
        assert_eq!(subtype(0x123456), 0);
    }
}
