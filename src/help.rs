//! The key list shown by prefix + `?`. Keep in step with the README tables.

pub struct HelpSection {
    pub title: &'static str,
    pub rows: &'static [(&'static str, &'static str)],
}

static SECTIONS: [HelpSection; 5] = [
    HelpSection {
        title: "sidebar",
        rows: &[
            ("j / k", "down / up, 3j works"),
            ("{ / }", "prev / next folder"),
            ("gg / G", "top / bottom"),
            ("enter", "open chat / fold folder"),
            ("n", "new chat here"),
            ("N", "new folder"),
            ("/", "filter, esc clears"),
            ("esc", "clear the filter"),
        ],
    },
    HelpSection {
        title: "everywhere",
        rows: &[
            ("alt+1..9", "jump N chats down"),
            ("alt+shift+1..9", "same, going up"),
            ("prefix, then hjkl", "move focus"),
            ("prefix, then z", "zoom pane"),
            ("prefix, then b", "hide / show sidebar"),
            ("prefix, then i", "environment panel"),
            ("prefix, then [", "scroll mode"),
            ("prefix, then ?", "this list"),
            ("prefix, then q", "quit"),
        ],
    },
    HelpSection {
        title: "scroll mode",
        rows: &[
            ("k / j", "line back / forward"),
            ("ctrl+u/d", "half page back / fwd"),
            ("pgup/dn", "full page back / fwd"),
            ("g / G", "oldest / newest"),
            ("q / esc", "leave scroll mode"),
        ],
    },
    HelpSection {
        title: "fork prompt",
        rows: &[("f", "open a copy (fork)"), ("esc", "cancel")],
    },
    HelpSection {
        title: "environment panel",
        rows: &[("prefix, then l", "focus, h/esc back"), ("j / k", "down / up"), ("enter", "toggle / open")],
    },
];

pub fn sections() -> &'static [HelpSection] {
    &SECTIONS
}
