use serde_json::{Map, Value};

use crate::core::i18n::tr;

/// A study task the column and the palette start with one click; each carries how Schrumbi wants it done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flow {
    Cards,
    Explain,
    Summary,
    Week,
    Inbox,
    Quiz,
    Setup,
}

impl Flow {
    pub const ALL: [Flow; 7] = [Flow::Cards, Flow::Explain, Flow::Summary, Flow::Week, Flow::Inbox, Flow::Quiz, Flow::Setup];

    /// The name of the matching MCP prompt, e.g. `/mcp__schrumbi__karten` in Claude Code.
    pub fn key(self) -> &'static str {
        match self {
            Flow::Cards => "karten",
            Flow::Explain => "erklaeren",
            Flow::Summary => "zusammenfassen",
            Flow::Week => "woche",
            Flow::Inbox => "inbox",
            Flow::Quiz => "abfragen",
            Flow::Setup => "einrichten",
        }
    }

    /// Whether the agent may change nothing during this turn; Schrumbi declines its writes.
    pub fn read_only(self) -> bool {
        matches!(self, Flow::Explain | Flow::Quiz)
    }

    /// What the agent is told on top of the user's words: which tools, in which order, and what not to do.
    pub fn guide(self) -> &'static str {
        match self {
            Flow::Cards => "Read the source first (read_pdf_text for the given pages, else read_note). Write atomic cards: one fact each, the front a short question, the back a short answer, LaTeX as $…$. Then call add_cards ONCE with all cards, into the note named under `Cards go to` (or get_context's cards_note). Do not write the cards anywhere else. Afterwards say in one line how many cards went where.",
            Flow::Explain => "Explain the given passage or page plainly and briefly, with one example if it helps; read neighbouring pages with read_pdf_text only when the passage needs them. Change nothing: this turn is read-only, the user can insert your answer into a note themselves.",
            Flow::Summary => "Read the whole PDF page by page with read_pdf_text (10 pages per call). Then create ONE new note with create_note in the module's notes folder: look at list_folder of that folder first and give `name` so the file follows the naming pattern of the notes already there. The note has short sections with the key points, definitions and formulas. Say where it went.",
            Flow::Week => "Look at list_deadlines (days 14), get_week and list_cards (due counts). Propose a short plan for the next seven days as a list: what to do on which day, the next deadline first, reviews of due cards in small daily blocks. Ask before creating anything; after the user agrees, create each planned task with add_deadline (kind task) in one go.",
            Flow::Inbox => "Call list_inbox and propose_filing. Show one short line per file: where it goes and its new name, using Schrumbi's guesses unless the content says otherwise. After the user agrees, move them with file_document, one call per file, all in this turn so they undo as one step. Never delete or overwrite.",
            Flow::Quiz => "Quiz the user on the given note or cards: one question at a time, wait for the answer, then say briefly what was right or missing and ask the next one. Never rate or change cards and never write anything; this turn is read-only.",
            Flow::Setup => "Help the user set up Schrumbi. Call read_setup first. If the study folder has material (list_folder, search, read_pdf_text of course plans or syllabi), derive the modules with folders and short codes, the lecture period and breaks, the naming pattern and Kürzel from existing file names, weekly deadlines from exercise sheets, and how the user works. If there is no material, ask a few short questions one at a time (semester and its dates, modules, how they want their files named) before proposing. Then call propose_setup ONCE with everything and end your turn; the user takes items from the card. Files are never renamed or moved: put such ideas into `renames` only.",
        }
    }

    /// What the agent hears as the user's words, in the user's language.
    pub fn prompt(self) -> String {
        match self {
            Flow::Cards => tr("Mach Karteikarten daraus und ergänze sie in der passenden Notiz."),
            Flow::Explain => tr("Erklär mir das in einfachen Worten."),
            Flow::Summary => tr("Fass diese Vorlesung als neue Notiz im Modulordner zusammen."),
            Flow::Week => tr("Plan mit mir die nächste Woche."),
            Flow::Inbox => tr("Räum meine Inbox auf: schlag für jede Datei Ordner und Namen vor."),
            Flow::Quiz => tr("Frag mich dazu ab, eine Frage nach der anderen."),
            Flow::Setup => tr("Richte Schrumbi mit mir ein."),
        }
    }

    /// The short label in menus, the palette and the empty column.
    pub fn label(self) -> String {
        match self {
            Flow::Cards => tr("Karten erstellen"),
            Flow::Explain => tr("Erklären"),
            Flow::Summary => tr("Vorlesung zusammenfassen"),
            Flow::Week => tr("Woche planen"),
            Flow::Inbox => tr("Inbox aufräumen"),
            Flow::Quiz => tr("Abfragen"),
            Flow::Setup => tr("Mit KI einrichten"),
        }
    }

    /// The icon next to the label.
    pub fn icon(self) -> &'static str {
        match self {
            Flow::Cards => "layers",
            Flow::Explain => "info",
            Flow::Summary => "file-text",
            Flow::Week => "calendar",
            Flow::Inbox => "inbox",
            Flow::Quiz => "graduation-cap",
            Flow::Setup => "settings",
        }
    }

    fn mcp_description(self) -> &'static str {
        match self {
            Flow::Cards => "Flashcards from a PDF page range or a note, added to the right note in Schrumbi",
            Flow::Explain => "Explain a passage of a PDF or note, read-only",
            Flow::Summary => "Summarize a lecture PDF into a new note in its module folder",
            Flow::Week => "Plan the next week from deadlines, due cards and the lecture week",
            Flow::Inbox => "Propose where inbox files go and file them after a yes",
            Flow::Quiz => "Quiz on a note or its cards, nothing is written or rated",
            Flow::Setup => "Set up Schrumbi from the study folder or by a few questions",
        }
    }

    fn mcp_arguments(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Flow::Cards | Flow::Explain => &[("path", "PDF or note, relative to the study folder; the open one when omitted"), ("pages", "For a PDF: a page or range like 3 or 3-5")],
            Flow::Summary | Flow::Quiz => &[("path", "PDF or note, relative to the study folder; the open one when omitted")],
            Flow::Week | Flow::Inbox | Flow::Setup => &[],
        }
    }

    /// The flow behind an MCP prompt name.
    pub fn from_key(key: &str) -> Option<Flow> {
        Flow::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// The MCP prompts Schrumbi offers terminal users: name, description and arguments.
pub fn mcp_prompts() -> Vec<(&'static str, &'static str, Vec<(&'static str, &'static str)>)> {
    Flow::ALL.into_iter().map(|f| (f.key(), f.mcp_description(), f.mcp_arguments().to_vec())).collect()
}

/// The text of an MCP prompt with its arguments filled in.
pub fn mcp_text(flow: Flow, args: &Map<String, Value>) -> String {
    let arg = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let mut target = String::new();
    if let Some(p) = arg("path") {
        target.push_str(&format!(" Source: {p}."));
    } else if !flow.mcp_arguments().is_empty() {
        target.push_str(" Source: what get_context reports as open in Schrumbi.");
    }
    if let Some(pages) = arg("pages") {
        target.push_str(&format!(" Pages: {pages}."));
    }
    format!("Use the schrumbi tools (call get_context first).{target} {} Answer in the language get_context reports.", flow.guide())
}
