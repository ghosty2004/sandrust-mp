//! ClientCheck (RPC 103): memory and collision-model checksums.

use super::*;

impl Behaviour {
    // ---- ClientCheck (RPC 103) ---------------------------------------------------------------------------------

    /// ClientCheck (RPC 103). Every reply is computed with samp's algorithm on this client's real state and memory:
    /// type 2 the entity flags word; 5 the checksum of gta_sa.exe memory as it is in this process (our patches
    /// included); 0x45 the checksum of this client's own module image (samp_client.dll, as samp reads its own);
    /// 0x46 / 0x47 GTA's model info / collision data; 0x48 the clock. Nothing is replayed or substituted: memory we
    /// changed answers as changed. Mirrors samp.dll 0x10011A60.
    pub fn client_check(&mut self, net: &mut NetClient, runner: &mut Runner, r: &ClientCheckRequest) {
        crate::log!(
            "behaviour: ClientCheck request type {:#x} arg {:#x} offset {} count {}",
            r.check_type,
            r.arg,
            r.offset,
            r.count
        );
        if !bh::client_check_in_limits(r.offset, r.count) {
            return;
        }
        let at = |base: u32| base.wrapping_add(u32::from(r.offset));
        let reply = match r.check_type {
            0x02 => {
                let ped = local_ped();
                let (mut arg, mut result) = (r.arg, 0u8);
                if ped != 0 {
                    let veh: u32 = rd(ped + PED_VEHICLE);
                    if in_vehicle(ped) && !is_passenger(ped) {
                        if veh != 0 {
                            arg = rd(veh + ENTITY_CHECK_WORD);
                            result = 2;
                        }
                    } else {
                        arg = rd(ped + ENTITY_CHECK_WORD);
                        result = 1;
                    }
                }
                Some((arg, result))
            }
            0x05 if bh::client_check_exe_range(r.arg) => checksum(at(r.arg), r.count).map(|c| (r.arg, c)),
            0x45 if r.arg <= bh::CLIENT_CHECK_MODULE_LIMIT => {
                checksum(at(crate::dll::module_base().wrapping_add(r.arg)), r.count).map(|c| (r.arg, c))
            }
            0x46 => {
                let mi = model_info(r.arg);
                let c = if mi != 0 { checksum(at(mi), r.count) } else { Some(0) };
                c.map(|c| (r.arg, c))
            }
            0x47 => {
                let mi = model_info(r.arg);
                let c = if mi != 0 { collision_checksum(runner, r.arg as i32, mi, r.offset, r.count) } else { Some(0) };
                c.map(|c| (r.arg, c))
            }
            0x48 => Some((bh::client_check_time_arg(qpc_ms()), 0)),
            _ => None,
        };
        if let Some((arg, result)) = reply {
            crate::log!("behaviour: ClientCheck reply type {:#x} arg {arg:#x} result {result:#04x}", r.check_type);
            net.send_rpc(&ClientCheckResponse { check_type: r.check_type, arg, result }, Reliability::ReliableOrdered);
        }
    }
}

/// `CModelInfo::ms_modelInfoPtrs[model]` for 0 ≤ model ≤ 20000 (samp's bounds), else 0.
fn model_info(model: u32) -> u32 {
    if model > game::MODEL_INFO_COUNT - 1 {
        return 0;
    }
    rd(game::MODEL_INFO_PTRS + 4 * model)
}

/// The checksum of `count` bytes at `addr` of this process; `None` (no reply) when they are not
/// readable (samp would fault there).
fn checksum(addr: u32, count: u16) -> Option<u8> {
    if addr == 0 || !crate::dll::readable(addr, usize::from(count)) {
        crate::log!("behaviour: ClientCheck: {addr:#x}+{count} not readable; no reply");
        return None;
    }
    let mut buf = vec![0u8; usize::from(count)];
    ProcessMemory.read(addr, &mut buf).ok()?;
    Some(bh::client_check_sum(&buf))
}

/// Type 0x47: the model is requested when it is not loaded (HAS_MODEL_LOADED polled up to 1000 times, `Sleep(1)`
/// apart), the checksum taken over its collision model (`CBaseModelInfo` +0x14), and the model marked as no longer
/// needed when nothing uses it (+8 reference count 0).
fn collision_checksum(runner: &mut Runner, model: i32, mi: u32, offset: u16, count: u16) -> Option<u8> {
    const MI_COL_MODEL: u32 = 0x14;
    const MI_REF_COUNT: u32 = 8;
    if !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        runner.run(op::REQUEST_MODEL, &[Arg::Int(model)]);
        runner.run(op::LOAD_ALL_MODELS_NOW, &[]);
        let mut polls = 0;
        while !runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            polls += 1;
            if polls >= bh::CLIENT_CHECK_LOAD_POLLS {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut result = 0;
    if runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
        let col: u32 = rd(mi + MI_COL_MODEL);
        if col != 0 {
            result = checksum(col.wrapping_add(u32::from(offset)), count)?;
        }
        if rd::<u16>(mi + MI_REF_COUNT) == 0 && runner.run(op::HAS_MODEL_LOADED, &[Arg::Int(model)]) {
            runner.run(op::MARK_MODEL_AS_NO_LONGER_NEEDED, &[Arg::Int(model)]);
        }
    }
    Some(result)
}
