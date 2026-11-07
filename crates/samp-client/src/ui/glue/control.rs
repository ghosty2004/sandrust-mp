//! Control-channel queries (may run on the background thread: they only read state or queue work).

use super::try_with_ui;

/// `chat [N]`.
pub fn control_chat(n: usize) -> String {
    try_with_ui(|st| st.core.chat_report(n)).unwrap_or_else(|| "chat count=0 error=busy".into())
}

/// `input`.
pub fn control_input() -> String {
    try_with_ui(|st| {
        let mut s = st.core.input_report();
        s.push_str(&format!(
            " cursor_mode={} renderer={}",
            st.lock.mode(),
            if st.renderer.is_some() {
                "ok"
            } else if st.renderer_failed {
                "failed"
            } else {
                "none"
            }
        ));
        if let Some(g) = &st.core.last_game_text {
            s.push_str(&format!(
                " gametext_style={} gametext_ms={} gametext={}",
                g.style,
                g.time_ms,
                crate::ui::core::quote(&g.text)
            ));
        }
        s
    })
    .unwrap_or_else(|| "error ui busy".into())
}

/// `bubbles`: the chat bubbles held per player.
pub fn control_bubbles() -> String {
    try_with_ui(|st| st.core.bubbles_report()).unwrap_or_else(|| "error ui busy".into())
}

/// `ui`: dialog, scoreboard, textdraws, menus, death window, bubbles, cursor mode.
pub fn control_ui() -> String {
    try_with_ui(|st| format!("{} cursor_mode={} tags={}", st.core.ui_report(), st.lock.mode(), st.remotes.len()))
        .unwrap_or_else(|| "error ui busy".into())
}

/// `say TEXT` (debug helper): queued, run on the next frame as typed input.
pub fn control_say(text: &str) -> String {
    try_with_ui(|st| {
        st.say_queue.push(text.as_bytes().to_vec());
        "say queued".to_string()
    })
    .unwrap_or_else(|| "error ui busy".into())
}
