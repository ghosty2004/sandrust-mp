//! Class selection state machine (player-state.md "128 PlayerRequestClass",
//! "129 PlayerRequestSpawn"), independent of the game so it can be tested.
//!
//! Rules from the spec:
//! * entering class selection: the first tick sends RPC 128 with the current
//!   index;
//! * Left/Right arrows are ignored for 2000 ms after entering, need ≥ 250 ms
//!   between presses and only work once the server answered the last request
//!   with "selectable"; Right wraps to 0 after the last class, Left wraps to
//!   `count − 1` before 0; each press clears "class info received" and sends
//!   RPC 128;
//! * Shift sends RPC 129 once 2000 ms have passed, class info was received
//!   and no spawn request is pending; it sets "request pending";
//! * RPC 129 response 2, or any non-zero response while a request is
//!   pending, runs the local spawn routine; otherwise "pending" is cleared.

use std::time::{Duration, Instant};

pub const ARROW_GUARD: Duration = Duration::from_millis(2000);
pub const ARROW_REPEAT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub left: bool,
    pub right: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Send RPC 128 with this class index (u32 on the wire).
    RequestClass(u32),
    /// Send RPC 129.
    RequestSpawn,
}

#[derive(Debug)]
pub struct ClassSelection {
    active: bool,
    /// Initial RPC 128 already sent for this selection round.
    announced: bool,
    entered_at: Instant,
    last_arrow: Instant,
    /// "class info received" (LocalPlayer+0x143 in the spec).
    info_received: bool,
    /// "spawn request pending" (LocalPlayer+0x302).
    request_pending: bool,
    index: u32,
    count: u32,
}

impl ClassSelection {
    pub fn new(now: Instant) -> Self {
        Self {
            active: false,
            announced: false,
            entered_at: now,
            last_arrow: now,
            info_received: false,
            request_pending: false,
            index: 0,
            count: 1,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn info_received(&self) -> bool {
        self.info_received
    }

    pub fn request_pending(&self) -> bool {
        self.request_pending
    }

    /// Enters class selection (after InitGame, or on a forced reselection);
    /// `count` is the server's class count from InitGame.
    pub fn enter(&mut self, now: Instant, count: u32) {
        self.active = true;
        self.announced = false;
        self.info_received = false;
        self.request_pending = false;
        self.count = count.max(1);
        if self.index >= self.count {
            self.index = 0;
        }
        self.entered_at = now;
        self.last_arrow = now;
    }

    /// Leaves class selection (done by the spawn routine).
    pub fn leave(&mut self) {
        self.active = false;
        self.request_pending = false;
    }

    /// One frame of class selection; `keys` are the currently held keys.
    pub fn tick(&mut self, now: Instant, keys: Keys) -> Option<Action> {
        if !self.active {
            return None;
        }
        if !self.announced {
            self.announced = true;
            return Some(Action::RequestClass(self.index));
        }
        if now.duration_since(self.entered_at) < ARROW_GUARD {
            return None;
        }
        if !self.request_pending {
            if !self.info_received {
                return None;
            }
            if keys.shift {
                self.request_pending = true;
                return Some(Action::RequestSpawn);
            }
        }
        if self.info_received && now.duration_since(self.last_arrow) > ARROW_REPEAT {
            if keys.left {
                self.last_arrow = now;
                self.index = if self.index == 0 { self.count - 1 } else { self.index - 1 };
                self.info_received = false;
                return Some(Action::RequestClass(self.index));
            }
            if keys.right {
                self.last_arrow = now;
                self.index = if self.index + 1 >= self.count { 0 } else { self.index + 1 };
                self.info_received = false;
                return Some(Action::RequestClass(self.index));
            }
        }
        None
    }

    /// RPC 128 response.
    pub fn on_class_response(&mut self, selectable: bool) {
        if selectable {
            self.info_received = true;
        }
    }

    /// RPC 129 response; returns true when the local spawn routine must run.
    pub fn on_spawn_response(&mut self, response: u8) -> bool {
        if response == 2 || (response != 0 && self.request_pending) {
            true
        } else {
            self.request_pending = false;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Keys = Keys { left: false, right: false, shift: false };
    const LEFT: Keys = Keys { left: true, right: false, shift: false };
    const RIGHT: Keys = Keys { left: false, right: true, shift: false };
    const SHIFT: Keys = Keys { left: false, right: false, shift: true };

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn first_tick_requests_current_class() {
        let t0 = Instant::now();
        let mut c = ClassSelection::new(t0);
        assert_eq!(c.tick(t0, NONE), None, "inactive");
        c.enter(t0, 3);
        assert_eq!(c.tick(t0, SHIFT), Some(Action::RequestClass(0)));
        assert_eq!(c.tick(ms(t0, 10), SHIFT), None);
    }

    #[test]
    fn guard_and_info_gate_shift() {
        let t0 = Instant::now();
        let mut c = ClassSelection::new(t0);
        c.enter(t0, 3);
        c.tick(t0, NONE);
        c.on_class_response(true);
        assert_eq!(c.tick(ms(t0, 1999), SHIFT), None, "2000 ms guard");
        assert_eq!(c.tick(ms(t0, 2000), SHIFT), Some(Action::RequestSpawn));
        assert!(c.request_pending());
        assert_eq!(c.tick(ms(t0, 2100), SHIFT), None, "pending");
        assert!(c.on_spawn_response(1));
    }

    #[test]
    fn shift_needs_class_info() {
        let t0 = Instant::now();
        let mut c = ClassSelection::new(t0);
        c.enter(t0, 1);
        c.tick(t0, NONE);
        assert_eq!(c.tick(ms(t0, 3000), SHIFT), None);
        c.on_class_response(false);
        assert_eq!(c.tick(ms(t0, 3001), SHIFT), None, "refused class keeps it blocked");
    }

    #[test]
    fn arrows_wrap_and_repeat_limit() {
        let t0 = Instant::now();
        let mut c = ClassSelection::new(t0);
        c.enter(t0, 3);
        c.tick(t0, NONE);
        c.on_class_response(true);
        assert_eq!(c.tick(ms(t0, 2500), LEFT), Some(Action::RequestClass(2)));
        // Blocked until the server answers.
        assert_eq!(c.tick(ms(t0, 2900), LEFT), None);
        c.on_class_response(true);
        assert_eq!(c.tick(ms(t0, 2700), RIGHT), None, "250 ms repeat");
        assert_eq!(c.tick(ms(t0, 2751), RIGHT), Some(Action::RequestClass(0)));
        c.on_class_response(true);
        assert_eq!(c.tick(ms(t0, 3100), RIGHT), Some(Action::RequestClass(1)));
    }

    #[test]
    fn spawn_responses() {
        let t0 = Instant::now();
        let mut c = ClassSelection::new(t0);
        c.enter(t0, 1);
        assert!(c.on_spawn_response(2), "force spawn");
        assert!(!c.on_spawn_response(1), "allowed but nothing pending");
        c.tick(t0, NONE);
        c.on_class_response(true);
        c.tick(ms(t0, 2000), SHIFT);
        assert!(!c.on_spawn_response(0));
        assert!(!c.request_pending());
    }
}
