//! The F5 network-statistics overlay: its values and the text samp draws from them.

/// Values of the F5 network-statistics overlay. Fields our client has no counter for stay 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetStats {
    pub download_kbs: f32,
    pub upload_kbs: f32,
    pub vehicles: u32,
    pub vehicle_models: u32,
    pub vehicle_models_loaded: u32,
    pub rw_objects: u32,
    pub ped_slots_used: u32,
    pub object_slots_used: u32,
    pub shot_comp_mode: u32,
    pub free_ides: u32,
    pub in_car: bool,
    pub send_rate: u32,
    pub streaming_mb: u32,
    pub streaming_total_mb: u32,
    pub system_mb: u32,
    pub process_kb: u32,
    pub working_set_kb: u32,
    pub messages_in_send_buffer: u32,
    pub messages_sent: u32,
    pub bytes_sent: u32,
    pub acks_sent: u32,
    pub acks_in_send_buffer: u32,
    pub messages_waiting_for_ack: u32,
    pub messages_resent: u32,
    pub bytes_resent: u32,
    pub packet_loss: f32,
    pub messages_received: u32,
    pub bytes_received: u32,
    pub acks_received: u32,
    pub duplicate_acks_received: u32,
}

/// The F5 text, line by line as samp concatenates it: rates, the pool block, the send rate, memory, then
/// the network statistics block. Every line ends with `\n`. Mirrors samp.dll 0x10060D70.
pub fn net_stats_text(s: &NetStats) -> Vec<u8> {
    let mut t = format!("Download Rate: {:.2} KB/s\nUpload Rate: {:.2} KB/s\n", s.download_kbs, s.upload_kbs);
    t += &format!(
        "Vehicles: {}\nVehicle Models: {}\nVehicle Models Loaded: {}\nRWObjects: {}\nPedSlotsUsed: {}\nObjectSlotsUsed: {}\nShotCompMode: {}\nFreeIDEs: {}\n",
        s.vehicles,
        s.vehicle_models,
        s.vehicle_models_loaded,
        s.rw_objects,
        s.ped_slots_used,
        s.object_slots_used,
        s.shot_comp_mode,
        s.free_ides
    );
    t += &format!("{} Send Rate: {}\n", if s.in_car { "InCar" } else { "OnFoot" }, s.send_rate);
    t += &format!("Streaming Mem: {}MB Total: {}MB System: {}MB\n", s.streaming_mb, s.streaming_total_mb, s.system_mb);
    t += &format!("Process Mem: {}KB Working Set: {}KB\n", s.process_kb, s.working_set_kb);
    t += &format!(
        "Messages in Send buffer: {}\nMessages sent: {}\nBytes sent: {}\nAcks sent: {}\nAcks in send buffer: {}\nMessages waiting for ack: {}\nMessages resent: {}\nBytes resent: {}\nPacketloss: {:.1}%\nMessages received: {}\nBytes received: {}\nAcks received: {}\nDuplicate acks received: {}\n",
        s.messages_in_send_buffer,
        s.messages_sent,
        s.bytes_sent,
        s.acks_sent,
        s.acks_in_send_buffer,
        s.messages_waiting_for_ack,
        s.messages_resent,
        s.bytes_resent,
        s.packet_loss,
        s.messages_received,
        s.bytes_received,
        s.acks_received,
        s.duplicate_acks_received
    );
    t.into_bytes()
}
