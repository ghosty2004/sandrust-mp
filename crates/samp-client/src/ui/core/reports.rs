//! Control-channel reports of the UI state: `ui`, `bubbles`, `chat` and `input`.

use samp_ui::chat::{ChatEntry, DisplayMode, EntryType};
use samp_ui::colour;

use super::{UiCore, quote};

impl UiCore {
    /// `ui` control report.
    pub fn ui_report(&self) -> String {
        let mut s = String::from("ui");
        match self.dialogs.current() {
            Some(d) => {
                let sel = d.selected().map_or(-1, |v| v as i32);
                s.push_str(&format!(
                    " dialog_open=1 dialog_id={} dialog_style={:?} dialog_rows={} dialog_columns={} dialog_selected={sel} dialog_text={}",
                    d.id,
                    d.style,
                    d.rows().len(),
                    d.columns(),
                    quote(d.edit.text())
                ));
            }
            None => s.push_str(" dialog_open=0"),
        }
        s.push_str(&format!(
            " responses={} scoreboard_open={} scoreboard_rows={} textdraws={} textdraw_ids={:?} select={} hovered={} menu={} deaths={} death_visible={} bubbles={} hidden_tags={}",
            self.responses_sent,
            self.scoreboard.is_open() as u8,
            self.scoreboard.rows().len(),
            self.textdraws.count(),
            self.textdraws.ids(),
            self.textdraws.selection.active as u8,
            self.textdraws.selection.hovered.map_or(-1, i32::from),
            self.menus.current().map_or(-1, i32::from),
            self.deaths.rows().count(),
            self.deaths.visible() as u8,
            (0..samp_ui::bubbles::SLOTS as u16).filter(|&i| self.bubbles.get(i).is_some()).count(),
            self.hidden_tags.len(),
        ));
        s
    }

    /// `bubbles` control reply: header + one line per active bubble, ascending player id. Same format
    /// as samp-tracer's `bubbles` for the original client, so the two can be diffed.
    pub fn bubbles_report(&self) -> String {
        let mut lines = Vec::new();
        for id in 0..samp_ui::bubbles::SLOTS as u16 {
            if let Some(b) = self.bubbles.get(id) {
                lines.push(format!(
                    "bubble id={id} colour={:#010x} distance={:.2} duration={} lines={} text={}",
                    b.colour,
                    b.distance,
                    b.duration_ms,
                    b.lines,
                    quote(&b.text)
                ));
            }
        }
        let mut out = format!("bubbles count={}", lines.len());
        for l in lines {
            out.push('\n');
            out.push_str(&l);
        }
        out
    }

    // ---- control channel -----------------------------------------------------------

    /// `chat [N]`: header plus the last N non-empty entries (oldest first), each with its colour runs as
    /// drawn (ui-rendering.md, "3.3 Embedded colour codes").
    pub fn chat_report(&self, n: usize) -> String {
        let all: Vec<&ChatEntry> = self.chat.entries().flatten().collect();
        let take = n.min(all.len());
        let visible: Vec<Option<&ChatEntry>> = self.chat.visible().collect();
        let mut out = format!(
            "chat count={take} total={} page_size={} scroll={} at_bottom={} mode={} timestamps={} visible_lines={}",
            all.len(),
            self.chat.page_size(),
            self.chat.scroll_pos(),
            self.chat.at_bottom() as u8,
            mode_num(self.chat.mode()),
            self.chat.timestamps() as u8,
            visible.iter().filter(|e| e.is_some()).count(),
        );
        for e in &all[all.len() - take..] {
            out.push('\n');
            out.push_str(&entry_line(e));
        }
        out
    }

    /// `input`: chat input state.
    pub fn input_report(&self) -> String {
        let e = self.input.edit();
        format!(
            "input open={} text={} caret={} history={} netgame={} connected={} page_size={} cfg={}",
            self.input.is_open() as u8,
            quote(e.text()),
            e.caret(),
            self.input.history().count(),
            self.netgame as u8,
            self.connected as u8,
            self.chat.page_size(),
            self.cfg_path().map(|p| quote(p.to_string_lossy().as_bytes())).unwrap_or_else(|| "-".into()),
        )
    }
}

fn mode_num(m: DisplayMode) -> u8 {
    match m {
        DisplayMode::Hidden => 0,
        DisplayMode::Plain => 1,
        DisplayMode::Outlined => 2,
    }
}

fn entry_line(e: &ChatEntry) -> String {
    let kind = match e.kind {
        EntryType::PlayerChat => 2,
        EntryType::Info => 4,
        EntryType::Debug => 8,
    };
    let runs: Vec<String> = colour::colour_runs(&e.text, e.text_colour)
        .iter()
        .map(|r| format!("{:08X}:{}", r.colour, quote(&r.text)))
        .collect();
    format!(
        "line type={kind} colour={:08X} name_colour={:08X} name={} text={} runs=[{}]",
        e.text_colour,
        e.name_colour,
        quote(&e.prefix()),
        quote(&e.text),
        runs.join(",")
    )
}
