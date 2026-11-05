//! Connection lifecycle of the netgame (milestone M10): samp's netgame state, the connect
//! timer and the reactions to RakNet's packets, RPC 40 and RPC 130.
//!
//! Static: main loop B 0x1000b5a0, connect tick B 0x10008930, packet handler B 0x1000b250,
//! lost connection B 0x1000ace0, server closed B 0x10008dd0, RPC 40 reset B 0x1000a530, RPC 130
//! B 0x10010520. Measured: `docs/phase3/trace-notes.md` "Connection lifecycle" (TRACE-8).
//!
//! Main loop per frame: the packet handler first; while the state is not 5 the "not connected"
//! scene; in state 1 the connect tick (more than 3000 ms since the last attempt → print
//! `Connecting to H:P...`, connect, remember the time once Connect has returned, state 2).

/// samp's netgame states (NetGame +0x3cd).
pub mod state {
    /// Waiting to connect (after construction, a failed attempt, full, lost).
    pub const WAIT_CONNECT: u32 = 1;
    /// Connect called.
    pub const CONNECTING: u32 = 2;
    /// InitGame received.
    pub const CONNECTED: u32 = 5;
    /// `0x22` accepted, waiting for InitGame.
    pub const AWAIT_JOIN: u32 = 6;
    /// RPC 40 reset: the RakNet connection stays, the client waits for InitGame.
    pub const RESTARTING: u32 = 11;
}

/// The connect tick fires when strictly more than this has passed since the last attempt
/// (B 0x10008942 `cmp eax, 3000; jbe`).
pub const CONNECT_INTERVAL_MS: u64 = 3000;

/// Chat texts (build B .rdata) and the helper each goes through: `Debug` = type 8
/// (B 0x100680b0), `Info` = type 4 (B 0x10068030).
pub mod msg {
    pub const DIDNT_RESPOND: &str = "The server didn't respond. Retrying..";
    pub const FULL: &str = "The server is full. Retrying...";
    pub const SERVER_CLOSED: &str = "Server closed the connection.";
    pub const LOST: &str = "Lost connection to the server. Reconnecting..";
    pub const JOINING: &str = "Connected. Joining the game...";
    pub const ENCRYPTION: &str = "Failed to initialize encryption.";
    pub const BANNED: &str = "You are banned from this server.";
    pub const WRONG_PASSWORD: &str = "Wrong server password.";
    pub const RESTARTING: &str = "The server is restarting..";
    pub const REJECT_VERSION: &str = "CONNECTION REJECTED: Incorrect Version.";
    pub const REJECT_NICK: &str = "CONNECTION REJECTED: Unacceptable NickName";
    pub const REJECT_NICK_HINTS: [&str; 3] = [
        "Please choose another nick between and 3-20 characters",
        "Please use only a-z, A-Z, 0-9",
        "Use /quit to exit or press ESC and select Quit Game",
    ];
    pub const REJECT_MOD: &str = "CONNECTION REJECTED: Bad mod version.";
    pub const REJECT_SLOT: &str = "CONNECTION REJECTED: Unable to allocate a player slot.";

    pub fn connecting(host: &str, port: u16) -> String {
        format!("Connecting to {host}:{port}...")
    }
}

/// What the host does, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Type-8 chat line.
    Debug(&'static str),
    /// Type-4 chat line.
    Info(&'static str),
    /// RakClient `Disconnect` (block time in ms; no notification goes out in these paths).
    Disconnect(u32),
    /// The RPC 40 reset routine (B 0x1000a530), which ends with the type-4 line
    /// "The server is restarting.." and leaves the state at 11.
    Reset,
    /// Delete every remote player from the pool (B 0x1000ad20 loop over 0..1004).
    DeletePlayers,
    /// Stop the audio stream; `wait` for its worker.
    StopAudio { wait: bool },
}

/// The netgame's connection state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetGame {
    pub state: u32,
    /// GetTickCount of the last connect (initially the construction time).
    last_attempt_ms: u64,
}

impl NetGame {
    /// Construction (B 0x1000b920 → state 1, last attempt = now; "Started" is printed there).
    pub fn new(now_ms: u64) -> Self {
        Self { state: state::WAIT_CONNECT, last_attempt_ms: now_ms }
    }

    pub fn connected(&self) -> bool {
        self.state == state::CONNECTED
    }

    /// The connect tick: true when the host must print `Connecting to …` and connect now; the
    /// host then reports the time after RakClient::Connect returned ([`Self::attempt_started`]).
    pub fn connect_due(&mut self, now_ms: u64) -> bool {
        self.state == state::WAIT_CONNECT && now_ms.saturating_sub(self.last_attempt_ms) > CONNECT_INTERVAL_MS
    }

    /// After Connect returned (A 0x10008985: GetTickCount is read again after the call): the
    /// attempt time and state 2.
    pub fn attempt_started(&mut self, now_ms: u64) {
        self.last_attempt_ms = now_ms;
        self.state = state::CONNECTING;
    }

    /// RPC 139 InitGame.
    pub fn on_init_game(&mut self) {
        self.state = state::CONNECTED;
    }

    /// RPC 40 (the reset itself is [`Action::Reset`]).
    pub fn on_server_restart(&mut self) -> Vec<Action> {
        self.state = state::RESTARTING;
        vec![Action::Reset]
    }

    /// A RakNet packet through the packet handler (B 0x1000b250).
    pub fn on_packet(&mut self, p: crate::netclient::NetPacket) -> Vec<Action> {
        use crate::netclient::NetPacket as P;
        match p {
            P::AttemptFailed => {
                self.state = state::WAIT_CONNECT;
                vec![Action::Debug(msg::DIDNT_RESPOND)]
            }
            P::ServerFull => {
                self.state = state::WAIT_CONNECT;
                vec![Action::Debug(msg::FULL)]
            }
            // B 0x10008dd0: no state change, no reset (TRACE-8 kick: the world stays as it was).
            P::ServerClosed => {
                vec![Action::Debug(msg::SERVER_CLOSED), Action::StopAudio { wait: true }, Action::Disconnect(2000)]
            }
            // B 0x1000ace0.
            P::ConnectionLost => {
                self.state = state::RESTARTING;
                let v = vec![
                    Action::Disconnect(0),
                    Action::Debug(msg::LOST),
                    Action::Reset,
                    Action::DeletePlayers,
                    Action::StopAudio { wait: true },
                ];
                self.state = state::WAIT_CONNECT;
                v
            }
            P::Accepted => {
                self.state = state::AWAIT_JOIN;
                vec![Action::Debug(msg::JOINING)]
            }
            P::EncryptionFailed => vec![Action::Debug(msg::ENCRYPTION)],
            P::Banned => vec![Action::Debug(msg::BANNED)],
            P::InvalidPassword => vec![Action::Debug(msg::WRONG_PASSWORD), Action::Disconnect(0)],
        }
    }

    /// RPC 130 ConnectionRejected (B 0x10010520): its lines (type 4), then Disconnect(500);
    /// the state is unchanged, so no retry follows.
    pub fn on_rejected(&mut self, reason: u8) -> Vec<Action> {
        let mut v = match reason {
            1 => vec![Action::Info(msg::REJECT_VERSION)],
            2 => {
                let mut v = vec![Action::Info(msg::REJECT_NICK)];
                v.extend(msg::REJECT_NICK_HINTS.iter().map(|h| Action::Info(h)));
                v
            }
            3 => vec![Action::Info(msg::REJECT_MOD)],
            4 => vec![Action::Info(msg::REJECT_SLOT)],
            _ => Vec::new(),
        };
        v.push(Action::Disconnect(500));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netclient::NetPacket as P;

    #[test]
    fn connect_tick_three_seconds_after_start_and_after_failures() {
        let mut n = NetGame::new(1000);
        assert!(!n.connect_due(4000), "exactly 3000 ms is not enough (jbe)");
        assert!(n.connect_due(4001));
        // Connect took 25 ms: the attempt time is taken after it.
        n.attempt_started(4026);
        assert_eq!(n.state, state::CONNECTING);
        assert!(!n.connect_due(20000), "only in state 1");
        // No answer after ~6.05 s: the next attempt starts in the same frame.
        assert_eq!(n.on_packet(P::AttemptFailed), vec![Action::Debug(msg::DIDNT_RESPOND)]);
        assert!(n.connect_due(10051));
        n.attempt_started(10076);
        // Full: answered at once, so the next attempt waits the rest of the 3 s from the end of
        // the previous Connect.
        assert_eq!(n.on_packet(P::ServerFull), vec![Action::Debug(msg::FULL)]);
        assert!(!n.connect_due(13076));
        assert!(n.connect_due(13077));
    }

    #[test]
    fn close_paths_match_the_measured_behaviour() {
        let mut n = NetGame::new(0);
        n.on_init_game();
        // Kick / server shutdown: line, audio stop, Disconnect(2000); still state 5, no retry.
        assert_eq!(
            n.on_packet(P::ServerClosed),
            vec![Action::Debug(msg::SERVER_CLOSED), Action::StopAudio { wait: true }, Action::Disconnect(2000)]
        );
        assert!(n.connected() && !n.connect_due(1_000_000));
        // Wrong password: no retry.
        let mut n = NetGame::new(0);
        assert!(n.connect_due(5000));
        n.attempt_started(5000);
        assert_eq!(n.on_packet(P::InvalidPassword), vec![Action::Debug(msg::WRONG_PASSWORD), Action::Disconnect(0)]);
        assert!(!n.connect_due(1_000_000));
        // Lost: reset + reconnect at once (last attempt long ago).
        let mut n = NetGame::new(0);
        assert!(n.connect_due(5000));
        n.attempt_started(5000);
        n.on_init_game();
        let a = n.on_packet(P::ConnectionLost);
        assert_eq!(a[0], Action::Disconnect(0));
        assert_eq!(a[1], Action::Debug(msg::LOST));
        assert_eq!(a[2], Action::Reset);
        assert!(a.contains(&Action::DeletePlayers));
        assert!(n.connect_due(60_000));
        // gmx: state 11, no reconnect until InitGame.
        let mut n = NetGame::new(0);
        n.on_init_game();
        assert_eq!(n.on_server_restart(), vec![Action::Reset]);
        assert_eq!(n.state, state::RESTARTING);
        assert!(!n.connect_due(1_000_000));
        n.on_init_game();
        assert!(n.connected());
        // Nick rejected: 4 type-4 lines, Disconnect(500), no retry.
        let mut n = NetGame::new(0);
        assert!(n.connect_due(5000));
        n.attempt_started(5000);
        let a = n.on_rejected(2);
        assert_eq!(a.len(), 5);
        assert_eq!(a[4], Action::Disconnect(500));
        assert!(!n.connect_due(1_000_000));
        assert_eq!(n.on_rejected(9), vec![Action::Disconnect(500)]);
    }
}
