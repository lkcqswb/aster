use crate::{
    agent::{self, Event, Running},
    composer::Edit,
    config::{Cli, Config},
    instructions,
    live2d::{Companion, Graphics, Shared},
    session::{Delivery, Entry, PendingMessage, Session, Store},
    tools,
};
use anyhow::{Context, Result, bail};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
        MouseEventKind,
    },
    execute,
};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph, Wrap},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

use crate::theme::{BG, DIM, FG, GOLD, JADE, LINE, RED};
const COMMANDS: &[(&str, &str)] = &[
    ("/new", "Start a fresh conversation"),
    ("/sessions", "Find and resume a session"),
    ("/rename", "Name this conversation"),
    ("/fork", "Branch the conversation"),
    ("/model", "Choose live MiniMax or demo"),
    (
        "/history",
        "Search and revisit earlier conversation entries",
    ),
    ("/context", "Inspect model context and attached files"),
    ("/skills", "Inspect available project and personal skills"),
    ("/skill", "Use a skill by name"),
    ("/prompts", "Browse reusable task prompts"),
    ("/prompt", "Use a reusable prompt"),
    ("/reload", "Refresh project instructions and resources"),
    ("/agents", "Inspect AGENTS.md instructions"),
    ("/files", "Browse and attach project files"),
    ("/find", "Find text and inspect matching lines"),
    ("/init", "Create project guidance if missing"),
    ("/plan", "Think and read · no edits"),
    ("/build", "Work with file and shell tools"),
    ("/permissions", "ask, allow, or deny actions"),
    ("/check", "Verify a file independently"),
    ("/checks", "Inspect current checks and their history"),
    ("/tasks", "Browse local project tests and build commands"),
    ("/task", "Run a named project task without a model request"),
    ("/run", "Run a local command without a model request"),
    ("/output", "Watch the latest command output"),
    ("/recover", "Ask for help with a failed command"),
    ("/together", "Open the companion action menu"),
    ("/work", "Open 弄玉's plan and task evidence"),
    ("/review", "Review the changes made this turn"),
    ("/steer", "Give a new direction during work"),
    ("/follow", "Queue the next task"),
    ("/queue", "Inspect waiting messages"),
    ("/next", "Run the next saved message"),
    ("/drop", "Remove a waiting message by ID"),
    ("/compact", "Summarize older context · local · auto on|off"),
    ("/limits", "Turn limits and the context window"),
    ("/checkpoint", "Inspect the latest context checkpoint"),
    ("/restore", "Restore archived context as a new conversation"),
    ("/export", "Save a readable transcript"),
    ("/tools", "Expand or collapse tool details"),
    (
        "/mood",
        "Set a profile emotion, optionally with strength 0–1",
    ),
    ("/emotion", "List or set a companion emotion"),
    ("/motion", "List or play a companion motion"),
    ("/look", "Ask 弄玉 to look toward you"),
    ("/pet", "Show or hide the companion"),
    ("/demo", "Run an offline file-and-check task"),
    ("/status", "Session, usage and renderer details"),
    ("/stop", "Interrupt the current turn"),
    ("/delete", "Delete this session after confirmation"),
    ("/help", "Commands and shortcuts"),
    ("/quit", "Save and leave"),
];

pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .replace('\t', "    ")
}
fn style(c: Color) -> Style {
    Style::default().fg(c).bg(BG)
}
fn line(s: impl Into<String>, c: Color) -> Line<'static> {
    Line::styled(s.into(), style(c))
}
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut result = vec![];
    for source in clean(text).split('\n') {
        let mut current = String::new();
        let mut used = 0;
        for c in source.chars() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + w > width.max(1) && !current.is_empty() {
                result.push(current);
                current = String::new();
                used = 0
            }
            current.push(c);
            used += w;
        }
        result.push(current);
    }
    result
}
fn wrap_prose(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut result = vec![];
    let mut code = false;
    for source in clean(text).split('\n') {
        if source.trim_start().starts_with("```") {
            code = !code;
            result.extend(wrap(source, width));
            continue;
        }
        if code || source.starts_with("    ") {
            result.extend(wrap(source, width));
            continue;
        }
        let mut current = String::new();
        for word in source.split_inclusive(' ') {
            if !current.trim().is_empty() && current.width() + word.trim_end().width() > width {
                result.push(current.trim_end().to_string());
                current.clear();
            }
            for c in word.chars() {
                let size = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                if current.width() + size > width && !current.is_empty() {
                    result.push(current.trim_end().to_string());
                    current.clear();
                }
                if c != ' ' || !current.is_empty() {
                    current.push(c);
                }
            }
        }
        result.push(current.trim_end().to_string());
    }
    result
}

fn entry_lines(e: &Entry, width: usize, show_tools: bool) -> Vec<Line<'static>> {
    let mut lines = vec![];
    if e.role == "tool"
        && !show_tools
        && (e.text.starts_with("update_plan ") || e.text.starts_with("ask_user "))
    {
        return lines;
    }
    match e.role.as_str() {
        "tool" => {
            let first = clean(e.text.lines().next().unwrap_or(""));
            let (glyph, color) = if first.contains("failed") || first.contains("declined") {
                ("✗", RED)
            } else if first.contains("verified") {
                ("✓", JADE)
            } else if first.contains("retry required") {
                ("◇", GOLD)
            } else {
                ("·", DIM)
            };
            let (name, rest) = first.split_once("  ").unwrap_or((first.as_str(), ""));
            lines.push(Line::from(vec![
                Span::styled(format!("  {glyph} "), style(color)),
                Span::styled(
                    name.to_string(),
                    style(if color == DIM { DIM } else { color }),
                ),
                Span::styled(
                    format!("  {}", rest.trim_start()),
                    style(if color == RED { RED } else { DIM }),
                ),
            ]));
            if show_tools {
                for text in wrap(&e.text, width.saturating_sub(6)).into_iter().skip(1) {
                    lines.push(Line::from(vec![
                        Span::styled("    │ ", style(LINE)),
                        Span::styled(text, style(DIM)),
                    ]));
                }
            }
        }
        "nongyu" => {
            lines.push(Line::from(vec![Span::styled(
                "弄玉",
                style(JADE).add_modifier(Modifier::BOLD),
            )]));
            for mut rendered in crate::richtext::markdown(&e.text, width.saturating_sub(2)) {
                rendered.spans.insert(0, Span::styled("  ", style(FG)));
                lines.push(rendered);
            }
            lines.push(line("", FG));
        }
        "you" => {
            for text in wrap_prose(&e.text, width.saturating_sub(3)) {
                lines.push(Line::from(vec![
                    Span::styled("▍ ", style(GOLD)),
                    Span::styled(text, style(FG)),
                ]));
            }
            lines.push(line("", FG));
        }
        _ => {
            for (i, text) in wrap_prose(&e.text, width.saturating_sub(4))
                .into_iter()
                .enumerate()
            {
                lines.push(Line::from(vec![
                    Span::styled(if i == 0 { "  ※ " } else { "    " }, style(GOLD)),
                    Span::styled(text, style(GOLD)),
                ]));
            }
            lines.push(line("", FG));
        }
    }
    lines
}
#[derive(Default)]
struct TranscriptLayout {
    key: Option<(String, String, usize, bool, usize)>,
    after_tool: bool,
    lines: Vec<Line<'static>>,
    starts: Vec<usize>,
    rendered_total: usize,
}
impl TranscriptLayout {
    fn update(&mut self, session: &Session, width: usize, show_tools: bool) -> bool {
        if self
            .key
            .as_ref()
            .is_some_and(|(id, stamp, w, tools, count)| {
                id == &session.id
                    && stamp == &session.updated
                    && *w == width
                    && *tools == show_tools
                    && *count == session.entries.len()
            })
        {
            return false;
        }
        self.lines.clear();
        self.starts.clear();
        self.after_tool = false;
        for entry in &session.entries {
            let lines = entry_lines(entry, width, show_tools);
            // Tool rows sit together; leave a breath before the next message.
            if !lines.is_empty() && self.after_tool && entry.role != "tool" {
                self.lines.push(line("", FG));
            }
            self.starts.push(self.lines.len());
            if !lines.is_empty() {
                self.after_tool = entry.role == "tool";
            }
            self.lines.extend(lines);
        }
        self.key = Some((
            session.id.clone(),
            session.updated.clone(),
            width,
            show_tools,
            session.entries.len(),
        ));
        true
    }
}
struct Approval {
    tool: String,
    preview: String,
    scroll: u16,
    answer: crossbeam_channel::Sender<bool>,
}
enum Popup {
    Tasks {
        catalog: crate::tasks::Catalog,
        query: String,
        index: usize,
        preview: bool,
        scroll: u16,
    },
    History(crate::history::History),
    Actions(crate::actions::Menu),
    Project(Box<crate::navigator::Navigator>),
    Resources {
        items: Vec<crate::context::Resource>,
        skills: bool,
        query: String,
        index: usize,
    },
    Redirect {
        input: String,
        previous: Option<Box<Popup>>,
    },
    Question {
        question: String,
        options: Vec<String>,
        input: String,
        answer: crossbeam_channel::Sender<String>,
    },
    Info {
        title: String,
        text: String,
        scroll: u16,
    },
    Sessions {
        items: Vec<Session>,
        query: String,
        index: usize,
        /// A session ID awaiting delete confirmation.
        confirm: Option<String>,
    },
    Delete,
    Approval(Approval),
}
pub struct App {
    cfg: Config,
    cli: Cli,
    store: Store,
    pub session: Session,
    composer: crate::composer::Editor,
    prompts: crate::composer::PromptHistory,
    composer_width: usize,
    stream: String,
    running: Option<Running>,
    popup: Option<Popup>,
    // Keep one decision alive while read-only views replace each other.
    inspection_return: Option<Box<Popup>>,
    /// Lines scrolled up from the latest message; 0 follows new output.
    scroll: usize,
    scroll_max: usize,
    page: usize,
    transcript: TranscriptLayout,
    jump_to: Option<usize>,
    selection: usize,
    show_tools: bool,
    companion: Option<Companion>,
    pub portrait: Shared,
    graphics: Graphics,
    image_area: Rect,
    work_area: Rect,
    action_rows: Vec<(Rect, crate::actions::Action)>,
    last_image: Option<(u64, Rect)>,
    mood: String,
    state: String,
    last_type: Instant,
    reaction: Option<(String, Instant)>,
    notice: String,
    notice_at: Instant,
    esc_at: Option<Instant>,
    suggestions_hidden: bool,
    turn_started: Option<Instant>,
    cell_px: (f32, f32),
    meter: Option<((String, usize, String), u64)>,
    quit: bool,
    quit_started: Option<Instant>,
}
impl App {
    pub fn new(cfg: Config, cli: Cli, store: Store) -> Result<Self> {
        let mut session = if let Some(id) = &cli.resume {
            store.load(id)?
        } else if cli.continue_last {
            store
                .list(&cfg.project)?
                .into_iter()
                .next()
                .unwrap_or_else(|| Session::new(cfg.project.clone(), cfg.model.clone(), cli.demo))
        } else {
            Session::new(cfg.project.clone(), cfg.model.clone(), cli.demo)
        };
        if session.project != cfg.project {
            bail!(
                "This session belongs to {}. Start Aster with --project pointing there.",
                session.project.display()
            )
        }
        if matches!(session.status.as_str(), "thinking" | "working" | "speaking") {
            session.status = "interrupted".into();
            session.add(
                "notice",
                "Previous work was interrupted. Inspect files before continuing.",
            );
        }
        store.save(&session)?;
        let graphics = Graphics::detect(&cli.graphics);
        let companion = if cli.no_live2d || graphics == Graphics::Off {
            None
        } else {
            Some(Companion::start(cfg.clone()))
        };
        let prompts = crate::composer::PromptHistory::with(recent_prompts(&store, &session));
        Ok(Self {
            cfg,
            cli,
            store,
            prompts,
            session,
            composer: crate::composer::Editor::default(),
            composer_width: 60,
            stream: String::new(),
            running: None,
            popup: None,
            inspection_return: None,
            scroll: 0,
            scroll_max: 0,
            page: 10,
            transcript: TranscriptLayout::default(),
            jump_to: None,
            selection: 0,
            show_tools: false,
            companion,
            portrait: Shared::default(),
            graphics,
            image_area: Rect::default(),
            work_area: Rect::default(),
            action_rows: vec![],
            last_image: None,
            mood: "neutral".into(),
            state: "idle".into(),
            last_type: Instant::now() - Duration::from_secs(5),
            reaction: None,
            notice: String::new(),
            notice_at: Instant::now(),
            esc_at: None,
            suggestions_hidden: false,
            turn_started: None,
            cell_px: cell_pixels(),
            meter: None,
            quit: false,
            quit_started: None,
        })
    }
    fn notify(&mut self, text: impl Into<String>) {
        self.notice = text.into();
        self.notice_at = Instant::now();
    }
    fn info(&mut self, title: &str, text: impl Into<String>) {
        self.inspect(Popup::Info {
            title: title.into(),
            text: text.into(),
            scroll: 0,
        });
    }
    fn show_actions(&mut self) {
        let decision = self.inspection_return.is_some()
            || matches!(
                self.popup,
                Some(
                    Popup::Approval(_)
                        | Popup::Question { .. }
                        | Popup::Redirect { .. }
                        | Popup::Delete
                )
            );
        self.inspect(Popup::Actions(crate::actions::Menu::new(
            &self.session.work,
            self.running.is_some(),
            decision,
        )));
    }
    fn click(&mut self, column: u16, row: u16) -> Result<()> {
        if let Some(Popup::Actions(menu)) = &mut self.popup
            && let Some((_, action)) = self
                .action_rows
                .iter()
                .find(|(area, _)| area.contains((column, row).into()))
            && let Some(index) = menu
                .filtered()
                .iter()
                .position(|choice| choice.action == *action)
        {
            menu.index = index;
            return self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        if self.image_area.contains((column, row).into())
            || self.work_area.contains((column, row).into())
        {
            if let Some(c) = &self.companion {
                c.motion("listening", &self.mood, true, false);
            }
            self.reaction = Some(("listening".into(), Instant::now()));
            self.show_actions();
        }
        Ok(())
    }
    fn begin_redirect(&mut self) {
        if self.running.is_none() || matches!(self.popup, Some(Popup::Redirect { .. })) {
            return;
        }
        if matches!(
            self.inspection_return.as_deref(),
            Some(Popup::Redirect { .. })
        ) {
            self.popup = self.inspection_return.take().map(|p| *p);
        } else {
            let previous = self
                .inspection_return
                .take()
                .or_else(|| self.popup.take().map(Box::new));
            self.popup = Some(Popup::Redirect {
                input: String::new(),
                previous,
            });
        }
        self.last_image = None;
    }
    fn inspect(&mut self, popup: Popup) {
        if matches!(
            self.popup,
            Some(
                Popup::Approval(_)
                    | Popup::Question { .. }
                    | Popup::Redirect { .. }
                    | Popup::Delete
            )
        ) {
            self.inspection_return = self.popup.take().map(Box::new);
        }
        self.popup = Some(popup);
        self.last_image = None;
    }
    fn clear_decision(&mut self) {
        fn clear(popup: Option<Popup>) -> Option<Popup> {
            match popup {
                Some(Popup::Approval(_) | Popup::Question { .. }) => None,
                Some(Popup::Redirect { input, previous }) => Some(Popup::Redirect {
                    input,
                    previous: clear(previous.map(|p| *p)).map(Box::new),
                }),
                other => other,
            }
        }
        self.popup = clear(self.popup.take());
        self.inspection_return = clear(self.inspection_return.take().map(|p| *p)).map(Box::new);
        self.last_image = None;
    }
    fn persist(&self) -> Result<()> {
        self.store.save(&self.session)
    }
    fn input_set(&mut self, text: impl Into<String>) {
        self.composer.set(text);
        self.selection = 0;
        self.suggestions_hidden = false;
    }
    fn paste(&mut self, text: &str) {
        let text = clean(text);
        if let Some(Popup::History(history)) = &mut self.popup {
            history.paste(&text);
        } else if let Some(Popup::Actions(menu)) = &mut self.popup {
            menu.paste(&text);
        } else if let Some(Popup::Project(nav)) = &mut self.popup {
            nav.paste(&text);
        } else if matches!(&self.popup, Some(Popup::Tasks { preview: true, .. })) {
            // Inspecting must not silently change the selected command.
        } else if let Some(
            Popup::Resources { query, index, .. }
            | Popup::Sessions { query, index, .. }
            | Popup::Tasks { query, index, .. },
        ) = &mut self.popup
        {
            if query.len() + text.len() <= 160 {
                query.push_str(&text.replace('\n', " "));
                *index = 0;
            }
        } else if let Some(Popup::Question { input, .. } | Popup::Redirect { input, .. }) =
            &mut self.popup
        {
            if input.len() + text.len() <= 2000 {
                input.push_str(&text);
            }
        } else if self.popup.is_none() && self.composer.text.len() + text.len() <= 32_000 {
            self.composer.insert(&text);
            self.prompts.reset();
            self.last_type = Instant::now();
        }
    }
    fn suggestions(&self) -> Vec<(&'static str, &'static str)> {
        let input = self.composer.text.trim();
        if self.suggestions_hidden || !input.starts_with('/') || input.contains(' ') {
            return vec![];
        };
        COMMANDS
            .iter()
            .copied()
            .filter(|(name, _)| name.starts_with(input))
            .collect()
    }
    fn busy_guard(&mut self) -> bool {
        if self.running.is_some() {
            self.notify("Let this turn finish, or press Esc to stop it first.");
            true
        } else {
            false
        }
    }
    fn new_session(&mut self, title: &str) -> Result<()> {
        self.session = Session::new(
            self.cfg.project.clone(),
            self.session.model.clone(),
            self.session.demo,
        );
        if !title.is_empty() {
            self.session.title = title.into()
        }
        self.follow_latest();
        self.meter = None;
        self.stream.clear();
        self.state = "idle".into();
        self.persist()
    }
    fn command(&mut self, text: &str) -> Result<()> {
        let (cmd, arg) = text
            .split_once(char::is_whitespace)
            .map(|(a, b)| (a, b.trim()))
            .unwrap_or((text, ""));
        if !matches!(
            cmd,
            "/stop"
                | "/quit"
                | "/exit"
                | "/help"
                | "/status"
                | "/mood"
                | "/emotion"
                | "/motion"
                | "/look"
                | "/pet"
                | "/tools"
                | "/work"
                | "/together"
                | "/checks"
                | "/output"
                | "/review"
                | "/steer"
                | "/follow"
                | "/queue"
                | "/context"
                | "/history"
                | "/tasks"
                | "/checkpoint"
                | "/skills"
                | "/files"
                | "/find"
                | "/prompts"
                | "/drop"
                | "/limits"
        ) && self.busy_guard()
        {
            return Ok(());
        }
        match cmd {
   "/context"=>{let catalog=crate::context::discover(&self.cfg.project);let rules=instructions::load(&self.cfg.project)?;let percent=self.context_percent();let estimate=self.meter.as_ref().map(|m|m.1).unwrap_or(0);self.info("Context beside 弄玉",format!("About {estimate} of {} tokens ({percent}%) · auto-compact {}\n{} provider messages · {} KB of saved content\n{} visible transcript entries · {} waiting messages\n\nAGENTS.md sources\n{}\n\n{} skills available · {} prompts available\n\nSkills used this turn\n{}\n\nAttached files this turn\n{}\n\nUse @path or @{{path with spaces}} to attach a project file.\nUse @path:10-30 for selected lines.\nFull skill text is loaded only on invocation or read_skill.\n/compact [note] saves a recoverable checkpoint. /checkpoint shows it.\nByte counts describe content, not exact model tokens; the token figure is calibrated from the provider's latest count.",self.cfg.limits.context_tokens,if !self.session.auto_compact||self.cfg.limits.auto_compact==0{"off".to_string()}else{format!("at {}%",self.cfg.limits.auto_compact)},self.session.messages.len(),serde_json::to_vec(&self.session.messages)?.len()/1000,self.session.entries.len(),self.session.pending.len(),rules.iter().map(|r|r.path.display().to_string()).collect::<Vec<_>>().join("\n"),catalog.skills.len(),catalog.prompts.len(),self.session.work.skills.join("\n"),self.session.work.context_files.join("\n")));},
   "/skills"=>{if arg.is_empty(){self.show_resources(true)}else{let v=crate::context::discover(&self.cfg.project).read_skill(arg,"SKILL.md",true)?;self.info("Skills beside 弄玉",format!("{}\n\n{}\n\n/skill {} request · invoke",v["directory"].as_str().unwrap_or(""),v["content"].as_str().unwrap_or(""),arg));}},
   "/prompts"=>self.show_resources(false),
   "/skill"|"/prompt"=>{if arg.is_empty(){bail!("Add a resource name and your request")};let invocation=format!("{cmd} {arg}");crate::context::prepare(&self.cfg.project,&invocation,&crate::context::discover(&self.cfg.project))?;self.submit(invocation)?;},
   "/reload"=>{let rules=instructions::load(&self.cfg.project)?;let catalog=crate::context::discover(&self.cfg.project);self.info("Project resources refreshed",format!("{} AGENTS.md files · {} skills · {} prompts\n\nResources are also reloaded at the start of each turn.\n{}",rules.len(),catalog.skills.len(),catalog.prompts.len(),catalog.warnings.join("\n")));},
   "/steer"=>self.enqueue(arg.into(),Delivery::Steer)?,
   "/follow"=>self.enqueue(arg.into(),Delivery::FollowUp)?,
   "/queue"=>{let text=if self.session.pending.is_empty(){"No messages waiting.\n\nWhile working: Enter adds a direction; Alt+Enter queues the next task.\n/steer MESSAGE · /follow MESSAGE".into()}else{format!("{}\n\n/next runs the next message when idle.\n/drop ID removes a waiting message.\nStopping preserves the queue; it does not run automatically after an error or restart.",self.session.pending.iter().map(|m|format!("{} · {}\n{}\n",m.id,if m.delivery==Delivery::Steer{"direction"}else{"next task"},m.text)).collect::<Vec<_>>().join("\n"))};self.info("Messages waiting for 弄玉",text);},
   "/next"=>self.run_next()?,
   "/drop"=>{let Some(index)=self.session.pending.iter().position(|m|m.id==arg)else{bail!("Use /queue to find the message ID")};let item=&self.session.pending[index];if item.delivery==Delivery::Steer&&let Some(r)=&self.running {let mut q=r.steering.lock().unwrap();let Some(at)=q.iter().position(|m|m.id==arg)else{bail!("That direction has already reached the agent")};q.remove(at);}self.session.pending.remove(index);self.persist()?;self.notify("Waiting message removed");},
   "/tasks"=>self.inspect(Popup::Tasks{catalog:crate::tasks::discover(&self.cfg.project)?,query:arg.into(),index:0,preview:false,scroll:0}),
   "/task"=>{if arg.is_empty(){bail!("Use /task followed by a name from /tasks")};self.submit(format!("/task {arg}"))?;},
   "/history"=>self.inspect(Popup::History(crate::history::History::new(&self.session,arg.into()))),
   "/together"=>self.show_actions(),
   "/work"=>self.show_work(),
   "/files"|"/find"=>self.inspect(Popup::Project(Box::new(crate::navigator::Navigator::new(self.cfg.project.clone(),if cmd=="/files"{crate::navigator::Mode::Files}else{crate::navigator::Mode::Search},arg.into())))),
   "/checks"=>self.info("Checks beside 弄玉",self.session.work.checks_summary()),
   "/run"=>{if arg.is_empty(){bail!("Use /run followed by a shell command")};self.submit(format!("/run {arg}"))?;},
   "/output"=>self.info("Command output · 弄玉",self.session.work.command.as_ref().map(|c|c.summary()).unwrap_or_else(||"No command has run in this turn.\nCommand output appears here while it runs.\nF4 opens this view.".into())),
   "/recover"=>{let command=self.session.work.command.as_ref().context("No failed command to recover")?;if command.running || (command.exit_code==Some(0)&&!command.stopped&&!command.timed_out){bail!("The latest command has not failed")};let prompt=format!("Help me recover from the latest command failure. Share a concise plan with update_plan. Inspect the actual output and relevant project files, explain the cause supported by evidence, then make a focused fix and run an appropriate check. Do not blindly repeat the same command. Respect my permission settings.\n\n{}",command.summary());self.submit(prompt)?;},
   "/review"=>{let text=if self.session.work.diffs.is_empty(){"No file edits recorded for this turn. Shell changes may require a git diff.\n\n/work shows the plan and actual checks.".into()}else{self.session.work.diffs.iter().map(|(_,d)|d.as_str()).collect::<Vec<_>>().join("\n\n")};self.info("Review changes · 弄玉",text);},
   "/new"|"/clear"=>self.new_session(arg)?,
   "/sessions"|"/resume"=>{if !arg.is_empty(){let s=self.store.load(arg)?;self.switch_session(s)?;}else{self.open_sessions()?;}},
   "/rename"=>{if arg.is_empty(){bail!("Use /rename followed by a title")};self.session.title=arg.chars().take(120).collect();self.session.updated=chrono::Utc::now().to_rfc3339();self.persist()?;},
   "/fork"=>{self.session=self.session.fork();if !arg.is_empty(){self.session.title=arg.into()};self.session.add("notice","Forked the conversation. This session shares the project files; no files were rolled back.");self.persist()?;self.notify("New branch saved. Project files are shared.");},
   "/model"|"/models"=>{match arg{""=>self.info("Choose an agent",format!("Current: {}\n\n/model live     MiniMax · real model\n/model demo     Offline scripted demo\n/model NAME     Use a specific MiniMax model\n\nModel changes take effect on the next message.",if self.session.demo{"offline demo"}else{&self.session.model})),"demo"=>{self.session.demo=true;self.notify("Offline demo · no API calls");},"live"=>{self.session.demo=false;self.notify(format!("MiniMax · {}",self.session.model));},name=>{if name.len()>120{bail!("Model name is too long")};self.session.model=name.into();self.session.demo=false;self.notify(format!("Model: {name}"));}}self.persist()?;},
   "/agents"=>{let rules=instructions::load(&self.cfg.project)?;self.info("Project instructions",if rules.is_empty(){"No AGENTS.md found. /init creates project guidance.".into()}else{instructions::format(&rules)});},
   "/init"=>{let p=self.cfg.project.join("AGENTS.md");if p.exists(){self.info("AGENTS.md",fs::read_to_string(p)?)}else{tools::path(&self.cfg.project,"AGENTS.md")?;crate::session::private_write(&p,b"# Project guidance\n\n- Inspect relevant files before making changes.\n- Keep changes focused on the requested task.\n- Run the project's relevant checks and report actual results.\n- Do not read or publish credentials.\n\n## Build and test\n\nAdd this project's build and test commands here.\n")?;self.notify("Created AGENTS.md. Use /agents to inspect it.");}},
   "/plan"=>{self.session.mode="plan".into();self.persist()?;self.notify("Plan mode · read and discuss, no writes or shell commands");},
   "/build"=>{self.session.mode="build".into();self.persist()?;self.notify("Build mode · tools follow your permission setting");},
   "/permissions"=>{if !matches!(arg,"ask"|"allow"|"deny"){bail!("Use /permissions ask, allow, or deny")};self.cli.permissions=arg.into();self.notify(format!("Permissions: {arg} · applies to file writes and shell commands"));},
   "/check"=>self.local_check(arg)?,

   "/compact"=>{match arg{"auto on"|"auto"=>{self.session.auto_compact=true;self.persist()?;self.notify(if self.cfg.limits.auto_compact==0{"Auto-compact is on for this conversation, but --auto-compact 0 disables it for this run".to_string()}else{format!("Auto-compact on · at {}% of {} tokens",self.cfg.limits.auto_compact,self.cfg.limits.context_tokens)});},"auto off"=>{self.session.auto_compact=false;self.persist()?;self.notify("Auto-compact off for this conversation · /compact still works");},_ if arg=="local"||arg.starts_with("local ")=>self.compact(arg.strip_prefix("local").unwrap_or("").trim())?,note=>self.compact_with_summary(note)?}},
   "/limits"=>self.info("Turn limits",format!("{}\n\nSet them when starting Aster, for example:\naster --max-requests 60 --turn-seconds 1800 --context-window 200000\nEnvironment variables: ASTER_MAX_REQUESTS, ASTER_MAX_TOOLS, ASTER_TURN_SECONDS,\nASTER_MAX_OUTPUT_TOKENS, ASTER_TURN_OUTPUT_TOKENS, ASTER_CONTEXT_WINDOW, ASTER_AUTO_COMPACT.\n\nDecision waits pause the timer (up to 15 minutes each). Errors never retry automatically.\nThese bound effort, not money: every request also uses input tokens.",self.cfg.limits.describe())),
   "/checkpoint"=>self.info("Context checkpoint · 弄玉",self.session.checkpoint.as_ref().map(|c|c.report()).unwrap_or_else(||"No checkpoint yet. /compact [note] archives full context and keeps bounded recent exchanges with local historical excerpts. It makes no model request.".into())),
   "/restore"=>{if arg.is_empty(){bail!("Use /restore followed by the ID shown in /checkpoint")};self.persist()?;self.session=self.store.restore_checkpoint(arg,&self.cfg.project)?;self.scroll=0;self.stream.clear();self.notify("Full context restored as a new conversation. Project files are shared.");},
   "/export"=>{let p=self.store.export(&self.session)?;self.notify(format!("Saved {}",p.display()));},
   "/tools"=>{self.show_tools = !self.show_tools;self.notify(if self.show_tools{"Tool details expanded"}else{"Tool details collapsed"});},
   "/mood"|"/emotion"|"/motion"=>{
       let profile=crate::companion::Profile::load(self.cfg.companion_profile.as_deref())?;
       let motion=cmd=="/motion";
       let names=if motion{profile.motions.keys().cloned().collect::<Vec<_>>()}else{profile.emotions.keys().cloned().collect::<Vec<_>>()};
       if arg.is_empty(){self.info(if motion{"Companion motions"}else{"Companion emotions"},format!("{}\n\n{} NAME [strength] · strength 0–1\n/pet info · supported parameters and last result\n/pet reset · clear custom expression, motion and gaze",names.join("\n"),cmd));}
       else {
           let parts=arg.split_whitespace().collect::<Vec<_>>();
           if parts.len()>2 || !names.iter().any(|n|n==parts[0]){bail!("Choose a configured name: {}",names.join(", "))};
           let strength=parts.get(1).map(|v|v.parse::<f64>()).transpose()?.unwrap_or(1.0);
           let command=if motion{crate::companion::Control::Motion{name:parts[0].into(),strength}}else{crate::companion::Control::Emotion{name:parts[0].into(),strength}};
           let companion=self.companion.as_ref().context("Live2D is hidden; use /pet on")?;
           companion.control(command)?;
           if !motion{self.mood=parts[0].into();}
           self.notify("Companion control queued · /pet info shows the renderer result");
       }
   },
   "/look"=>{if let Some(c)=&self.companion{c.motion(&self.state,&self.mood,false,true)}self.reaction=Some(("listening".into(),Instant::now()));},
   "/pet"=>{match arg { "info" => {self.info("Companion interface v1",serde_json::to_string_pretty(&json!({"api":self.portrait.info["api"],"control":self.portrait.info["control"],"results":self.portrait.info["control_results"]}))?);}, "reset" => {self.companion.as_ref().context("Live2D is hidden; use /pet on")?.control(crate::companion::Control::Reset)?;self.mood="neutral".into();}, "off" => {self.companion=None;self.portrait=Shared::default();}, "on"|"retry"|"restart" => {self.companion=None;self.portrait=Shared::default();self.companion=Some(Companion::start(self.cfg.clone()));}, "" if self.companion.is_some() => {self.companion=None;self.portrait=Shared::default();}, "" => {self.companion=Some(Companion::start(self.cfg.clone()));}, _ => self.notify("Use /pet on, off, retry, info or reset") }self.last_image=None;},
   "/demo"=>{self.session.demo=true;self.submit(if arg=="work"{"companion demo".into()}else if arg.starts_with("evidence"){format!("evidence demo {}",arg.strip_prefix("evidence").unwrap_or(""))}else if arg.starts_with("command"){format!("command demo {}",arg.strip_prefix("command").unwrap_or(""))}else{"demo task".into()})?;},
   "/status"=>self.info("Aster · session status",format!("Session    {}\nProject    {}\nModel      {}\nProvider   {}\nMode       {} · permissions {}\nUsage      {} input / {} output tokens\nTools      {}\nChecks     {} passed / {} total\n\nGraphics   {}\nLive2D     {}\nFrames     {}\n\n{}\n\nTurn limits\n{}\nDecision waits pause the timer (up to 15 minutes each).\nNo automatic retries. Token limits are not a currency budget.",self.session.id,self.cfg.project.display(),self.session.model,if self.session.demo{"scripted demo"}else{"MiniMax"},self.session.mode,self.cli.permissions,self.session.input_tokens,self.session.output_tokens,self.session.tools,self.session.checks.iter().filter(|c|c.passed).count(),self.session.checks.len(),self.graphics.name(),self.portrait.status,self.portrait.frames,serde_json::to_string_pretty(&self.portrait.info)?,self.cfg.limits.describe())),
   "/stop"=>self.stop(),
   "/delete"=>self.popup=Some(Popup::Delete),
   "/help"=>self.info("Make yourself at home",format!("{}\n\nWRITING\nEnter send · Ctrl+J, Shift+Enter or \\ Enter new line\n↑↓ move between lines, then through earlier requests\nAlt/Ctrl+←→ or Alt+B/F word · Home/End line · Ctrl+A/E line\nCtrl+W or Alt+Backspace delete word · Ctrl+U/K delete to line start/end\nEsc Esc clears the draft (↑ brings it back) · Ctrl+C clears, then quits\n\nREADING\nPgUp/PgDn page · Shift+↑↓ or wheel 3 lines · Ctrl+Home top · Ctrl+End or Esc latest\nCtrl+O shows tool details · F7 searches history\n\nWORKING\nWhile 弄玉 works: Enter steers · Alt+Enter queues · Ctrl+G redirects · Esc stops\nCtrl+P sessions (Enter open · Ctrl+N new · Ctrl+D delete)\nF1 or click 弄玉 for local task controls · F2 plan · F3 review · F4 output\nF5 checks · F6 files · F7 history · F8 tasks\n\nThe model is an AI companion. Speaking motion follows text activity; no voice is synthesized.",COMMANDS.iter().map(|(a,b)|format!("{a:15} {b}")).collect::<Vec<_>>().join("\n"))),
   "/quit"|"/exit"=>self.request_quit(),
   _=>bail!("Unknown command. Type / to see available commands."),
  }
        Ok(())
    }
    /// A checkpoint whose summary is written by the model, in the background.
    fn compact_with_summary(&mut self, note: &str) -> Result<()> {
        if self.busy_guard() {
            return Ok(());
        }
        if note.len() > 2000 {
            bail!("Checkpoint note exceeds 2 KB");
        }
        if self.session.messages.is_empty() {
            bail!("No conversation context to checkpoint");
        }
        self.persist()?;
        self.running = Some(agent::spawn_compaction(
            self.session.clone(),
            note.into(),
            self.cfg.clone(),
            false,
        ));
        self.state = "thinking".into();
        self.session.work.activity = "Summarizing earlier context".into();
        self.turn_started = Some(Instant::now());
        self.notify(if self.session.demo {
            "Compacting · offline demo summary"
        } else {
            "Compacting · 弄玉 is summarizing the earlier conversation (one bounded request)"
        });
        Ok(())
    }
    fn compact(&mut self, note: &str) -> Result<()> {
        let Some(prepared) = crate::compaction::prepare(&self.session, note)? else {
            self.notify("Context is already short. Nothing to compact.");
            return Ok(());
        };
        self.session = self.store.checkpoint(&self.session, prepared)?;
        self.command("/checkpoint")
    }
    fn show_work(&mut self) {
        let text = if self.session.work.goal.is_empty() {
            "Tell me the task and I will keep its plan, changes and evidence here.\n\n/plan       discuss and inspect\n/build      make changes with tools\n/review     inspect this turn's edits\n/check      independently verify a file\n\nDuring work, approvals and questions appear beside me. F2 opens this card; Esc stops ongoing work.".into()
        } else {
            format!(
                "{}\nF3 /review · inspect edits\nF5 /checks · inspect evidence\nEsc closes this card · Esc again stops work",
                self.session.work.summary()
            )
        };
        self.info("Working together · 弄玉", text);
    }
    fn local_check(&mut self, argument: &str) -> Result<()> {
        let (path, expected) = argument
            .split_once(' ')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((argument, None));
        if path.is_empty() {
            bail!("Use /check path [expected JSON]");
        }
        let (kind, expected) = if let Some(text) = expected {
            serde_json::from_str::<Value>(text)?;
            ("json_equals", text)
        } else {
            ("exists", "")
        };
        let args = json!({"path":path,"kind":kind,"expected":expected});
        let result = tools::execute(
            &self.cfg.project,
            "check_file",
            &args,
            &Arc::new(AtomicBool::new(false)),
        );
        let (result, error) = match result {
            Ok(result) => {
                self.session
                    .checks
                    .push(serde_json::from_value(result.clone())?);
                (result, false)
            }
            Err(error) => (json!({"error":error.to_string(),"executed":true}), true),
        };
        if self.session.work.goal.is_empty() {
            self.session.work = crate::work::Work::begin(&format!("Check {path}"));
        }
        self.session
            .work
            .record("check_file", &args, &result, error);
        self.session.tools += 1;
        self.session.add("you", format!("/check {argument}"));
        self.session
            .add("tool", format!("check_file  {path}\n{result}"));
        let id = format!("local-check-{}", uuid::Uuid::new_v4().simple());
        self.session.messages.extend([
            json!({"role":"user","content":format!("Run this local file check: /check {argument}")}),
            json!({"role":"assistant","content":[{"type":"tool_use","id":id,"name":"check_file","input":args}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":result.to_string(),"is_error":error}]}),
        ]);
        self.reaction = Some((
            if self.session.work.verified() {
                "pleased"
            } else {
                "concerned"
            }
            .into(),
            Instant::now(),
        ));
        self.notify(self.session.work.verdict());
        self.persist()
    }
    fn show_resources(&mut self, skills: bool) {
        let catalog = crate::context::discover(&self.cfg.project);
        let items = if skills {
            &catalog.skills
        } else {
            &catalog.prompts
        };
        if items.is_empty() {
            self.info(
                if skills {
                    "Skills beside 弄玉"
                } else {
                    "Reusable prompts"
                },
                catalog.describe(skills),
            );
            return;
        }
        if !catalog.warnings.is_empty() {
            self.notify(format!(
                "{} discovery notes · /reload to inspect",
                catalog.warnings.len()
            ));
        }
        self.inspect(Popup::Resources {
            items: items.clone(),
            skills,
            query: String::new(),
            index: 0,
        });
        self.last_image = None;
    }
    fn enqueue(&mut self, text: String, delivery: Delivery) -> Result<()> {
        if text.trim().is_empty() {
            bail!("Add the message you want to send");
        }
        if self.running.is_none() {
            return self.submit(text);
        }
        if self.session.pending.len() >= 8
            || text.len()
                + self
                    .session
                    .pending
                    .iter()
                    .map(|m| m.text.len())
                    .sum::<usize>()
                > 32_000
        {
            bail!("Queue limit: eight messages and 32 KB total. Use /queue to review it.");
        }
        let message = PendingMessage::new(text, delivery.clone());
        self.session.pending.push(message.clone());
        if let Err(e) = self.persist() {
            self.session.pending.pop();
            return Err(e);
        }
        if delivery == Delivery::Steer {
            if let Some(r) = &self.running {
                r.steering.lock().unwrap().push_back(message);
            }
            self.notify("Direction saved · read at the next tool boundary");
        } else {
            self.notify("Next task saved · starts after the current turn finishes");
        }
        Ok(())
    }
    fn run_next(&mut self) -> Result<()> {
        if self.running.is_some() {
            bail!("Work is still running; Esc stops it");
        }
        if self.session.pending.is_empty() {
            self.notify("No messages waiting");
            return Ok(());
        }
        let next = self.session.pending.remove(0);
        if let Err(e) = self.submit(next.text.clone()) {
            self.session.pending.insert(0, next);
            return Err(e);
        }
        Ok(())
    }
    fn submit(&mut self, prompt: String) -> Result<()> {
        if self.busy_guard() {
            return Ok(());
        }
        if prompt.trim().is_empty() {
            return Ok(());
        }
        if prompt.len() > 32_000 {
            bail!("Message exceeds 32 KB")
        }
        let before = self.session.clone();
        let mut submitted = before.clone();
        submitted.add("you", &prompt);
        submitted
            .messages
            .push(json!({"role":"user","content":prompt}));
        submitted.status = "thinking".into();
        submitted.work = crate::work::Work::begin(&prompt);
        self.store.save(&submitted)?;
        self.session = submitted;
        self.running = Some(agent::spawn(
            before,
            prompt,
            self.cfg.clone(),
            self.cli.permissions.clone(),
        ));
        self.stream.clear();
        self.follow_latest();
        self.state = "thinking".into();
        self.turn_started = Some(Instant::now());
        self.notice.clear();
        Ok(())
    }
    fn stop(&mut self) {
        if let Some(r) = &self.running {
            r.cancel.store(true, Ordering::Relaxed);
            self.notify("Stopping… an already submitted request may still consume tokens.");
        } else {
            self.notify("Nothing is running.");
        }
    }
    fn request_quit(&mut self) {
        if self.running.is_some() {
            self.stop();
            self.quit_started.get_or_insert_with(Instant::now);
        } else {
            self.quit = true;
        }
    }
    fn tick(&mut self) -> Result<()> {
        if let Some(Popup::History(history)) = &mut self.popup {
            history.refresh(&self.session);
        }
        if let Some(Popup::Project(nav)) = &mut self.popup {
            nav.tick();
        }
        let mut advance_queue = false;
        let events = self
            .running
            .as_ref()
            .map(|r| r.events.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::DecisionClosed => {
                    self.clear_decision();
                }
                Event::Compacted {
                    automatic,
                    method,
                    before_bytes,
                    after_bytes,
                    ..
                } => {
                    self.notify(format!(
                        "{} · {} → {} KB · {} · /checkpoint",
                        if automatic {
                            "Context auto-compacted"
                        } else {
                            "Context compacted"
                        },
                        before_bytes / 1000,
                        after_bytes / 1000,
                        match method.as_str() {
                            "model" => "model summary",
                            "demo" => "demo summary",
                            _ => "local excerpts",
                        }
                    ));
                }
                Event::InputConsumed(id) => {
                    self.session.pending.retain(|m| m.id != id);
                    self.notify("New direction received");
                    if let Some(c) = &self.companion {
                        c.motion("listening", &self.mood, true, false);
                    }
                }
                Event::Work(work) => {
                    self.session.work = *work;
                    if let Some(Popup::Info { title, text, .. }) = &mut self.popup {
                        if title.starts_with("Command output") {
                            *text = self
                                .session
                                .work
                                .command
                                .as_ref()
                                .map(|c| c.summary())
                                .unwrap_or_default();
                        }
                        if title.starts_with("Working together") {
                            *text = self.session.work.summary();
                        }
                        if title.starts_with("Checks beside") {
                            *text = self.session.work.checks_summary();
                        }
                    }
                }
                Event::Question {
                    question,
                    options,
                    answer,
                } => {
                    self.clear_decision();
                    self.state = "waiting".into();
                    self.popup = Some(Popup::Question {
                        question,
                        options,
                        input: String::new(),
                        answer,
                    });
                    self.last_image = None;
                }
                Event::Delta(text) => {
                    self.state = "speaking".into();
                    self.stream.push_str(&text);
                }
                Event::State(state) => self.state = state,
                Event::Entry(role, text) => {
                    self.stream.clear();
                    self.session.add(&role, text);
                }
                Event::Usage(input, output) => {
                    self.session.input_tokens = input;
                    self.session.output_tokens = output;
                }
                Event::Approval {
                    tool,
                    preview,
                    answer,
                } => {
                    self.clear_decision();
                    self.state = "waiting".into();
                    self.popup = Some(Popup::Approval(Approval {
                        tool,
                        preview,
                        scroll: 0,
                        answer,
                    }));
                    self.last_image = None;
                }
                Event::Checkpoint(session) => {
                    let pending = std::mem::take(&mut self.session.pending);
                    self.session = *session;
                    self.session.pending = pending;
                    self.persist()?;
                }
                Event::Finished(session) => {
                    advance_queue = session.status == "done";
                    let happy = session.status == "done" && session.work.verified();
                    self.reaction = Some((
                        if happy {
                            "pleased"
                        } else if session.status == "error" {
                            "concerned"
                        } else {
                            "idle"
                        }
                        .into(),
                        Instant::now(),
                    ));
                    let pending = std::mem::take(&mut self.session.pending);
                    self.session = *session;
                    self.session.pending = pending;
                    if self.session.work.has_failures() {
                        self.notify("A check or command failed · F4 output · /work details");
                        self.reaction = Some(("concerned".into(), Instant::now()));
                    } else if self.session.work.has_stale_checks() {
                        self.notify("Edits changed the project after checks · F5 to review");
                        self.reaction = Some(("concerned".into(), Instant::now()));
                    }
                    self.running = None;
                    self.turn_started = None;
                    self.stream.clear();
                    self.state = "idle".into();
                    self.clear_decision();
                    self.persist()?;
                    if self.quit_started.is_some() {
                        self.quit = true;
                    }
                }
            }
        }
        if advance_queue
            && !self.quit
            && self.quit_started.is_none()
            && !self.session.pending.is_empty()
        {
            self.run_next()?;
        }
        if let Some(Popup::Actions(menu)) = &mut self.popup {
            menu.refresh(
                &self.session.work,
                self.running.is_some(),
                self.inspection_return.is_some(),
            );
        }
        if self
            .quit_started
            .is_some_and(|t| t.elapsed() > Duration::from_secs(2))
        {
            self.session.status = "interrupted".into();
            self.session.add(
                "notice",
                "Interrupted before the provider finished. Check project state before continuing.",
            );
            self.persist()?;
            self.quit = true;
        }
        let state = if self.inspection_return.is_some() {
            "reading"
        } else if !self.session.work.waiting.is_empty() && self.running.is_some() {
            "waiting"
        } else if self.running.is_some() {
            if self.state == "working" && self.session.work.activity == "Checking the result" {
                "checking"
            } else if self.state == "working" && self.session.work.activity == "Reading the project"
            {
                "reading"
            } else {
                self.state.as_str()
            }
        } else if matches!(&self.popup,Some(Popup::Project(nav)) if nav.reading() || nav.busy())
            || matches!(&self.popup, Some(Popup::History(history)) if history.preview)
            || matches!(&self.popup, Some(Popup::Tasks { preview: true, .. }))
        {
            "reading"
        } else if self.last_type.elapsed() < Duration::from_secs(2)
            && !self.composer.text.is_empty()
        {
            "listening"
        } else if self.session.work.has_failures() || self.session.work.has_stale_checks() {
            "concerned"
        } else if let Some((reaction, at)) = &self.reaction {
            if at.elapsed() < Duration::from_secs(3) {
                reaction
            } else {
                "idle"
            }
        } else {
            "idle"
        };
        if let Some(c) = &self.companion {
            c.motion(state, &self.mood, false, false);
            self.portrait = c.current();
        }
        Ok(())
    }
    fn key(&mut self, key: KeyEvent) -> Result<()> {
        let result = self.key_inner(key);
        if self.popup.is_none() && self.inspection_return.is_some() {
            self.popup = self.inspection_return.take().map(|p| *p);
            self.last_image = None;
        }
        result
    }
    fn key_inner(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind == KeyEventKind::Release {
            return Ok(());
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // With a draft and nothing else open, the first Ctrl+C clears the draft.
            if self.popup.is_none() && self.running.is_none() && !self.composer.is_empty() {
                let draft = self.composer.text.clone();
                self.prompts.push(&draft);
                self.input_set("");
                self.notify("Draft cleared · ↑ brings it back · Ctrl+C again saves and quits");
            } else {
                self.request_quit();
            }
            return Ok(());
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('g')
            && self.running.is_some()
            && !matches!(self.popup, Some(Popup::Redirect { .. }))
        {
            self.begin_redirect();
            return Ok(());
        }
        if key.code == KeyCode::F(1) && !matches!(self.popup, Some(Popup::Resources { .. })) {
            if matches!(self.popup, Some(Popup::Actions(_))) {
                self.popup = None;
            } else {
                self.show_actions();
            }
            return Ok(());
        }
        if key.code == KeyCode::F(8) {
            self.command("/tasks")?;
            return Ok(());
        }
        if key.code == KeyCode::F(7) {
            self.command("/history")?;
            return Ok(());
        }
        if key.code == KeyCode::F(2) {
            self.show_work();
            return Ok(());
        }
        if key.code == KeyCode::F(4) {
            self.command("/output")?;
            return Ok(());
        }
        if key.code == KeyCode::F(5) {
            self.command("/checks")?;
            return Ok(());
        }
        if key.code == KeyCode::F(6) {
            self.command("/files")?;
            return Ok(());
        }
        if key.code == KeyCode::F(3) {
            self.command("/review")?;
            return Ok(());
        }
        if let Some(popup) = self.popup.take() {
            match popup {
                Popup::Tasks {
                    catalog,
                    mut query,
                    mut index,
                    mut preview,
                    mut scroll,
                } => {
                    let filtered = catalog
                        .tasks
                        .iter()
                        .filter(|task| {
                            format!("{} {}", task.name, task.description)
                                .to_lowercase()
                                .contains(&query.to_lowercase())
                        })
                        .collect::<Vec<_>>();
                    match key.code {
                        KeyCode::Esc if preview => {
                            preview = false;
                            scroll = 0;
                        }
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Enter => {
                            if let Some(task) = filtered.get(index) {
                                if self.running.is_some() || self.inspection_return.is_some() {
                                    self.notify("Finish or stop the current task and resolve its decision before starting another.");
                                } else {
                                    self.submit(format!("/task {}", task.name))?;
                                    return Ok(());
                                }
                            }
                        }
                        KeyCode::Tab => {
                            preview = !preview;
                            scroll = 0;
                        }
                        KeyCode::Up | KeyCode::PageUp if preview => {
                            scroll = scroll.saturating_sub(4)
                        }
                        KeyCode::Down | KeyCode::PageDown if preview => {
                            scroll = scroll.saturating_add(4)
                        }
                        KeyCode::Up => index = index.saturating_sub(1),
                        KeyCode::Down => index = (index + 1).min(filtered.len().saturating_sub(1)),
                        KeyCode::PageUp => index = index.saturating_sub(5),
                        KeyCode::PageDown => {
                            index = (index + 5).min(filtered.len().saturating_sub(1))
                        }
                        KeyCode::Home => index = 0,
                        KeyCode::End => index = filtered.len().saturating_sub(1),
                        KeyCode::Char('u')
                            if key.modifiers.contains(KeyModifiers::CONTROL) && !preview =>
                        {
                            query.clear();
                            index = 0;
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && !preview
                                && query.len() < 160 =>
                        {
                            query.push(c);
                            index = 0;
                        }
                        KeyCode::Backspace if !preview => {
                            query.pop();
                            index = 0;
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Tasks {
                        catalog,
                        query,
                        index,
                        preview,
                        scroll,
                    });
                }
                Popup::History(mut history) => {
                    match history.key(key) {
                        crate::history::Action::Keep => self.popup = Some(Popup::History(history)),
                        crate::history::Action::Close => {}
                        crate::history::Action::Jump(entry) => {
                            if self.inspection_return.is_some() {
                                self.notify("A decision is still pending. Enter reads the selected entry here.");
                                self.popup = Some(Popup::History(history));
                            } else {
                                if history.items[entry].role == "tool" {
                                    self.show_tools = true;
                                }
                                self.jump_to = Some(entry);
                                self.notify("Earlier conversation · Page Down reads on · Ctrl+End returns to latest");
                            }
                        }
                    }
                }
                Popup::Actions(mut menu) => {
                    let filtered = menu.filtered();
                    match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Enter => {
                            if let Some(choice) = filtered.get(menu.index) {
                                match choice.action {
                                    crate::actions::Action::Return => {}
                                    crate::actions::Action::Command(command) => {
                                        self.command(command)?
                                    }
                                    crate::actions::Action::Redirect => self.begin_redirect(),
                                    crate::actions::Action::Stop => self.stop(),
                                }
                                return Ok(());
                            }
                        }
                        KeyCode::Down => {
                            menu.index = (menu.index + 1).min(filtered.len().saturating_sub(1))
                        }
                        KeyCode::Up => menu.index = menu.index.saturating_sub(1),
                        KeyCode::PageDown => {
                            menu.index = (menu.index + 5).min(filtered.len().saturating_sub(1))
                        }
                        KeyCode::PageUp => menu.index = menu.index.saturating_sub(5),
                        KeyCode::Home => menu.index = 0,
                        KeyCode::End => menu.index = filtered.len().saturating_sub(1),
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            menu.query.clear();
                            menu.index = 0;
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && menu.query.len() < 160 =>
                        {
                            menu.query.push(c);
                            menu.index = 0;
                        }
                        KeyCode::Backspace => {
                            menu.query.pop();
                            menu.index = 0;
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Actions(menu));
                }
                Popup::Project(mut nav) => {
                    match nav.key(key) {
                        crate::navigator::Action::Keep => self.popup = Some(Popup::Project(nav)),
                        crate::navigator::Action::Close => {}
                        crate::navigator::Action::Notice(message) => {
                            self.notify(message);
                            self.popup = Some(Popup::Project(nav));
                        }
                        crate::navigator::Action::Attach(reference) => {
                            if self.composer.text.len() + reference.len() + 1 > 32_000 {
                                self.notify(
                                    "The draft is full. Shorten it before attaching a file.",
                                );
                                self.popup = Some(Popup::Project(nav));
                            } else {
                                let text = if self.composer.text.trim().is_empty() {
                                    format!("{reference} ")
                                } else {
                                    format!("{} {reference} ", self.composer.text.trim_end())
                                };
                                self.input_set(text);
                                self.last_type = Instant::now();
                                self.notify(
                                    "Reference added to your draft · Enter sends it to 弄玉",
                                );
                            }
                        }
                    }
                    return Ok(());
                }
                Popup::Resources {
                    items,
                    skills,
                    mut query,
                    mut index,
                } => {
                    let filtered = items
                        .iter()
                        .filter(|r| {
                            format!("{} {}", r.name, r.description)
                                .to_lowercase()
                                .contains(&query.to_lowercase())
                        })
                        .collect::<Vec<_>>();
                    match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Down => index = (index + 1).min(filtered.len().saturating_sub(1)),
                        KeyCode::Up => index = index.saturating_sub(1),
                        KeyCode::PageDown => {
                            index = (index + 5).min(filtered.len().saturating_sub(1))
                        }
                        KeyCode::PageUp => index = index.saturating_sub(5),
                        KeyCode::Home => index = 0,
                        KeyCode::End => index = filtered.len().saturating_sub(1),
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            query.clear();
                            index = 0;
                        }
                        KeyCode::Enter => {
                            if let Some(resource) = filtered.get(index) {
                                let prepared = format!(
                                    "/{} {} {}",
                                    if skills { "skill" } else { "prompt" },
                                    resource.name,
                                    self.composer.text
                                );
                                if prepared.len() > 32_000 {
                                    self.notify(
                                        "The draft is full. Shorten it before choosing a resource.",
                                    );
                                    self.popup = Some(Popup::Resources {
                                        items,
                                        skills,
                                        query,
                                        index,
                                    });
                                    return Ok(());
                                }
                                self.input_set(prepared);
                                self.notify("Add your request, then Enter to send");
                            }
                            return Ok(());
                        }
                        KeyCode::F(1) => {
                            if let Some(resource) = filtered.get(index) {
                                let catalog = crate::context::discover(&self.cfg.project);
                                let text = if skills {
                                    catalog.read_skill(&resource.name, "SKILL.md", true)?["content"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string()
                                } else {
                                    catalog.prompt(&resource.name, "$ARGUMENTS")?
                                };
                                self.info(
                                    &format!("Resource · {}", resource.name),
                                    format!("{}\n\n{text}", resource.path.display()),
                                );
                            }
                            return Ok(());
                        }
                        KeyCode::Backspace => {
                            query.pop();
                            index = 0;
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && query.len() < 160 =>
                        {
                            query.push(c);
                            index = 0;
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Resources {
                        items,
                        skills,
                        query,
                        index,
                    });
                }
                Popup::Redirect {
                    mut input,
                    previous,
                } => {
                    match key.code {
                        KeyCode::Esc => {
                            self.popup = previous.map(|p| *p);
                            return Ok(());
                        }
                        KeyCode::Enter if !input.trim().is_empty() => {
                            if let Err(error) = self.enqueue(input.trim().into(), Delivery::Steer) {
                                self.popup = Some(Popup::Redirect { input, previous });
                                return Err(error);
                            }
                            return Ok(());
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            input.clear()
                        }
                        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let mut editor = crate::composer::Editor::new(input);
                            editor.key(key, usize::MAX);
                            input = editor.text;
                        }
                        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            input.push('\n')
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && input.len() < 4000 =>
                        {
                            input.push(c)
                        }
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Redirect { input, previous });
                }
                Popup::Question {
                    question,
                    options,
                    mut input,
                    answer,
                } => {
                    match key.code {
                        KeyCode::Esc => {
                            let _ = answer.send(String::new());
                            return Ok(());
                        }
                        KeyCode::Enter if !input.trim().is_empty() => {
                            let _ = answer.send(input.trim().into());
                            return Ok(());
                        }
                        KeyCode::Char(c)
                            if input.is_empty()
                                && c.is_ascii_digit()
                                && c != '0'
                                && (c as usize - '1' as usize) < options.len() =>
                        {
                            let _ = answer.send(options[c as usize - '1' as usize].clone());
                            return Ok(());
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            input.clear()
                        }
                        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let mut editor = crate::composer::Editor::new(input);
                            editor.key(key, usize::MAX);
                            input = editor.text;
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && input.len() < 2000 =>
                        {
                            input.push(c)
                        }
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Question {
                        question,
                        options,
                        input,
                        answer,
                    });
                }
                Popup::Approval(mut a) => match key.code {
                    KeyCode::Down | KeyCode::PageDown => {
                        a.scroll =
                            a.scroll
                                .saturating_add(if key.code == KeyCode::Down { 1 } else { 12 });
                        self.popup = Some(Popup::Approval(a));
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        a.scroll =
                            a.scroll
                                .saturating_sub(if key.code == KeyCode::Up { 1 } else { 12 });
                        self.popup = Some(Popup::Approval(a));
                    }
                    KeyCode::Char('y') => {
                        let _ = a.answer.send(true);
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        let _ = a.answer.send(false);
                    }
                    _ => self.popup = Some(Popup::Approval(a)),
                },
                Popup::Delete => {
                    if key.code == KeyCode::Char('y') {
                        let old = self.session.id.clone();
                        self.new_session("")?;
                        self.store.delete(&old)?;
                        self.notify("Session deleted. Project files were not changed.");
                    } else if !matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
                        self.popup = Some(Popup::Delete)
                    }
                }
                Popup::Info {
                    title,
                    text,
                    mut scroll,
                } => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => return Ok(()),
                        KeyCode::Down => scroll = scroll.saturating_add(1),
                        KeyCode::Up => scroll = scroll.saturating_sub(1),
                        KeyCode::PageDown | KeyCode::Char(' ') => {
                            scroll = scroll.saturating_add(12)
                        }
                        KeyCode::PageUp => scroll = scroll.saturating_sub(12),
                        KeyCode::Home => scroll = 0,
                        KeyCode::End => scroll = u16::MAX,
                        _ => {}
                    }
                    self.popup = Some(Popup::Info {
                        title,
                        text,
                        scroll,
                    });
                }
                Popup::Sessions {
                    items,
                    mut query,
                    mut index,
                    confirm,
                } => {
                    let filtered = filter_sessions(&items, &query);
                    let last = filtered.len().saturating_sub(1);
                    let control = key.modifiers.contains(KeyModifiers::CONTROL);
                    if let Some(id) = confirm {
                        if key.code == KeyCode::Char('y') && id != self.session.id {
                            self.store.delete(&id)?;
                            self.notify("Conversation deleted. Project files were not changed.");
                            let items = self.store.list(&self.cfg.project)?;
                            let index =
                                index.min(filter_sessions(&items, &query).len().saturating_sub(1));
                            self.popup = Some(Popup::Sessions {
                                items,
                                query,
                                index,
                                confirm: None,
                            });
                        } else {
                            self.popup = Some(Popup::Sessions {
                                items,
                                query,
                                index,
                                confirm: None,
                            });
                        }
                        return Ok(());
                    }
                    match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Down => index = if index >= last { 0 } else { index + 1 },
                        KeyCode::Up => index = if index == 0 { last } else { index - 1 },
                        KeyCode::PageDown => index = (index + 5).min(last),
                        KeyCode::PageUp => index = index.saturating_sub(5),
                        KeyCode::Home => index = 0,
                        KeyCode::End => index = last,
                        KeyCode::Enter => {
                            if let Some(s) = filtered.get(index)
                                && s.id != self.session.id
                            {
                                let chosen = (*s).clone();
                                self.switch_session(chosen)?;
                            }
                            return Ok(());
                        }
                        KeyCode::Char('n') if control => {
                            self.new_session("")?;
                            self.notify("New conversation · the previous one is saved");
                            return Ok(());
                        }
                        KeyCode::Char('d') if control => {
                            let target = filtered.get(index).map(|s| s.id.clone());
                            let confirm = match target {
                                Some(id) if id == self.session.id => {
                                    self.notify("Use /delete for the conversation you are in.");
                                    None
                                }
                                other => other,
                            };
                            self.popup = Some(Popup::Sessions {
                                items,
                                query,
                                index,
                                confirm,
                            });
                            return Ok(());
                        }
                        KeyCode::Char('u') if control => {
                            query.clear();
                            index = 0;
                        }
                        KeyCode::Backspace => {
                            query.pop();
                            index = 0
                        }
                        KeyCode::Char(c)
                            if !control
                                && !key.modifiers.contains(KeyModifiers::ALT)
                                && query.len() < 160 =>
                        {
                            query.push(c);
                            index = 0
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Sessions {
                        items,
                        query,
                        index,
                        confirm: None,
                    });
                }
            }
            return Ok(());
        }
        self.composer_key(key)
    }
    fn scroll_by(&mut self, lines: isize) {
        let next = (self.scroll as isize).saturating_add(lines).max(0) as usize;
        self.scroll = next.min(self.scroll_max);
        if self.scroll == 0 {
            self.jump_to = None;
        }
    }
    fn follow_latest(&mut self) {
        self.scroll = 0;
        self.jump_to = None;
    }
    fn open_sessions(&mut self) -> Result<()> {
        let items = self.store.list(&self.cfg.project)?;
        // The conversation you are in is rarely the one you want to open.
        let index = items
            .iter()
            .position(|s| s.id != self.session.id)
            .unwrap_or(0);
        self.popup = Some(Popup::Sessions {
            items,
            query: String::new(),
            index,
            confirm: None,
        });
        Ok(())
    }
    fn switch_session(&mut self, mut session: Session) -> Result<()> {
        if session.project != self.cfg.project {
            bail!("Session belongs to a different project")
        }
        self.persist()?;
        if matches!(session.status.as_str(), "thinking" | "working" | "speaking") {
            session.status = "interrupted".into();
            session.add(
                "notice",
                "Previous work was interrupted. Inspect files before continuing.",
            );
        }
        self.session = session;
        self.follow_latest();
        self.stream.clear();
        self.state = "idle".into();
        self.meter = None;
        self.persist()?;
        self.notify(format!(
            "Resumed · {}",
            tools::clip(&clean(&self.session.title), 60)
        ));
        Ok(())
    }
    fn escape(&mut self) {
        let palette = !self.suggestions().is_empty();
        if palette {
            // Esc cancels the command palette it opened, and never a longer draft.
            if !self.composer.text.trim().contains(char::is_whitespace)
                && self.composer.text.trim().len() <= 16
            {
                self.input_set("");
            } else {
                self.suggestions_hidden = true;
            }
            return;
        }
        if self.running.is_some() {
            self.stop();
            return;
        }
        if self.scroll > 0 {
            self.follow_latest();
            self.notice.clear();
            return;
        }
        if self.composer.is_empty() {
            return;
        }
        if self
            .esc_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(1500))
        {
            let draft = self.composer.text.clone();
            self.prompts.push(&draft);
            self.input_set("");
            self.esc_at = None;
            self.notify("Draft cleared · ↑ brings it back");
        } else {
            self.esc_at = Some(Instant::now());
            self.notify("Esc again clears the draft");
        }
    }
    fn composer_key(&mut self, key: KeyEvent) -> Result<()> {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if key.code != KeyCode::Esc {
            self.esc_at = None;
        }
        match key.code {
            KeyCode::PageUp => {
                self.scroll_by(self.page as isize);
                return Ok(());
            }
            KeyCode::PageDown => {
                self.scroll_by(-(self.page as isize));
                return Ok(());
            }
            KeyCode::Home if control => {
                self.scroll = self.scroll_max;
                return Ok(());
            }
            KeyCode::End if control => {
                self.follow_latest();
                self.notice.clear();
                return Ok(());
            }
            KeyCode::Up if shift && !alt => {
                self.scroll_by(3);
                return Ok(());
            }
            KeyCode::Down if shift && !alt => {
                self.scroll_by(-3);
                return Ok(());
            }
            _ => {}
        }
        if control {
            match key.code {
                KeyCode::Char('p') => {
                    if !self.busy_guard() {
                        self.open_sessions()?;
                    }
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('o') => {
                    self.show_tools = !self.show_tools;
                    self.notify(if self.show_tools {
                        "Tool details expanded · Ctrl+O collapses"
                    } else {
                        "Tool details collapsed"
                    });
                    return Ok(());
                }
                KeyCode::Char('j') => {
                    self.composer.insert("\n");
                    self.last_type = Instant::now();
                    return Ok(());
                }
                KeyCode::Char('l') => {
                    self.last_image = None;
                    return Ok(());
                }
                _ => {}
            }
        }
        let suggestions = self.suggestions();
        let chosen = self.selection.min(suggestions.len().saturating_sub(1));
        match key.code {
            KeyCode::Enter if alt => {
                let input = self.composer.text.trim().to_string();
                self.enqueue(input.clone(), Delivery::FollowUp)?;
                self.prompts.push(&input);
                self.input_set("");
            }
            KeyCode::Enter if shift => {
                self.composer.insert("\n");
            }
            KeyCode::Enter => {
                if !suggestions.is_empty() && self.composer.text.trim() != suggestions[chosen].0 {
                    self.input_set(suggestions[chosen].0);
                    return Ok(());
                }
                // A trailing backslash continues the message on a new line.
                if self.composer.text[..self.composer.cursor].ends_with('\\') {
                    let at = self.composer.cursor - 1;
                    self.composer.text.replace_range(at..at + 1, "\n");
                    return Ok(());
                }
                let input = self.composer.text.trim().to_string();
                if input.is_empty() {
                    return Ok(());
                }
                self.prompts.push(&input);
                if self.running.is_some() && !input.starts_with('/') {
                    self.enqueue(input, Delivery::Steer)?;
                    self.input_set("");
                    return Ok(());
                }
                self.input_set("");
                if input.starts_with('/') {
                    self.command(&input)?
                } else {
                    self.submit(input)?
                }
            }
            KeyCode::Tab => {
                if let Some((cmd, _)) = suggestions.get(chosen) {
                    self.input_set(format!("{cmd} "));
                }
            }
            KeyCode::Esc => self.escape(),
            KeyCode::Up if !suggestions.is_empty() => {
                self.selection = chosen.checked_sub(1).unwrap_or(suggestions.len() - 1)
            }
            KeyCode::Down if !suggestions.is_empty() => {
                self.selection = if chosen + 1 >= suggestions.len() {
                    0
                } else {
                    chosen + 1
                }
            }
            _ => {
                let before = self.composer.clone();
                match self.composer.key(key, self.composer_width) {
                    Edit::AboveTop => {
                        if let Some(text) = self.prompts.older(&self.composer.text) {
                            self.composer.set(text);
                            self.suggestions_hidden = true;
                        }
                    }
                    Edit::BelowBottom => {
                        if let Some(text) = self.prompts.newer() {
                            self.composer.set(text);
                            self.suggestions_hidden = true;
                        }
                    }
                    Edit::Handled if self.composer.text != before.text => {
                        if self.composer.text.len() > 32_000 {
                            self.composer = before;
                            self.notify("The draft is limited to 32 KB.");
                        } else {
                            self.prompts.reset();
                            self.last_type = Instant::now();
                            self.selection = 0;
                            self.suggestions_hidden = false;
                        }
                    }
                    Edit::Handled | Edit::Ignored => {}
                }
            }
        }
        Ok(())
    }
    fn context_percent(&mut self) -> u64 {
        let key = (
            self.session.id.clone(),
            self.session.messages.len(),
            self.session.updated.clone(),
        );
        let tokens = match &self.meter {
            Some((cached, tokens)) if *cached == key => *tokens,
            _ => {
                let tokens = self
                    .session
                    .context_estimate(16_000 + agent::TOOL_SCHEMA_BYTES);
                self.meter = Some((key, tokens));
                tokens
            }
        };
        tokens * 100 / self.cfg.limits.context_tokens.max(1)
    }
    pub fn draw(&mut self, f: &mut Frame) {
        let all = f.area();
        f.render_widget(Block::default().style(style(FG)), all);
        self.image_area = Rect::default();
        self.work_area = Rect::default();
        if all.width < 45 || all.height < 12 {
            f.render_widget(
                Paragraph::new("Aster needs at least 45 × 12 cells.").style(style(DIM)),
                all,
            );
            return;
        }
        let area = all.inner(Margin {
            horizontal: if all.width >= 80 { 2 } else { 1 },
            vertical: if all.height >= 30 { 1 } else { 0 },
        });
        self.draw_header(f, Rect::new(area.x, area.y, area.width, 1));
        f.render_widget(
            Paragraph::new("─".repeat(area.width as usize)).style(style(LINE)),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
        let columns = area.width.saturating_sub(6).max(1) as usize;
        self.composer_width = columns;
        let rows = self.composer.rows(columns);
        let visible_rows = rows.len().clamp(1, 6) as u16;
        let composer_h = visible_rows + 2;
        let footer_y = area.bottom().saturating_sub(1);
        let composer_y = footer_y.saturating_sub(composer_h);
        let body = Rect::new(
            area.x,
            area.y + 2,
            area.width,
            composer_y.saturating_sub(area.y + 2),
        );
        let pet_width = if self.companion.is_some() || self.portrait.frame.is_some() {
            if area.width >= 110 {
                area.width * 34 / 100
            } else if area.width >= 72 {
                (area.width * 30 / 100).max(24)
            } else {
                0
            }
        } else {
            0
        };
        let chat = Rect::new(
            body.x,
            body.y,
            body.width
                .saturating_sub(pet_width + if pet_width > 0 { 3 } else { 0 }),
            body.height,
        );
        if self.session.entries.is_empty() && self.stream.is_empty() && self.running.is_none() {
            self.welcome(f, chat)
        } else {
            self.conversation(f, chat)
        }
        if pet_width > 0 {
            let divider = chat.right() + 1;
            for y in body.y..body.bottom() {
                f.render_widget(
                    Paragraph::new("│").style(style(LINE)),
                    Rect::new(divider, y, 1, 1),
                );
            }
            let pet = Rect::new(body.right() - pet_width, body.y, pet_width, body.height);
            self.draw_pet(f, pet);
        }
        let composer = Rect::new(area.x, composer_y, area.width, composer_h);
        self.draw_composer(f, composer, columns, &rows, visible_rows as usize);
        self.draw_footer(f, Rect::new(area.x, footer_y, area.width, 1));
        let options = self.suggestions();
        if !options.is_empty() && self.popup.is_none() {
            let count = options.len().min(8) as u16;
            let w = area.width.min(76);
            let r = Rect::new(area.x, composer_y.saturating_sub(count + 2), w, count + 2);
            f.render_widget(Clear, r);
            f.render_widget(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(style(LINE))
                    .style(style(FG))
                    .title_bottom(Line::styled(
                        " ↑↓ choose · Tab complete · Esc close ",
                        style(DIM),
                    )),
                r,
            );
            if !r.intersection(self.image_area).is_empty() {
                self.image_area = Rect::default();
            }
            let chosen = self.selection.min(options.len() - 1);
            let start = chosen.saturating_sub(7);
            for (i, (name, help)) in options.iter().skip(start).take(8).enumerate() {
                let selected = i + start == chosen;
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!(" {} {:14}", if selected { "›" } else { " " }, name),
                            style(if selected { JADE } else { FG }),
                        ),
                        Span::styled(format!(" {help}"), style(DIM)),
                    ])),
                    Rect::new(r.x + 1, r.y + i as u16 + 1, r.width - 2, 1),
                );
            }
        }
        self.action_rows.clear();
        if let Some(popup) = &mut self.popup {
            let decision = matches!(
                popup,
                Popup::Approval(_)
                    | Popup::Actions(_)
                    | Popup::History(_)
                    | Popup::Tasks { .. }
                    | Popup::Project(_)
                    | Popup::Question { .. }
                    | Popup::Redirect { .. }
                    | Popup::Resources { .. }
            ) || matches!(popup, Popup::Info{title,..} if title.starts_with("Working together") || title.starts_with("Review changes") || title.starts_with("Messages waiting") || title.starts_with("Context beside") || title.starts_with("Skills beside") || title.starts_with("Command output") || title.starts_with("Checks beside") || title.starts_with("Context checkpoint"));
            let side_by_side = decision && pet_width > 0 && chat.width >= 42;
            // Nothing half-hidden behind a panel: wide characters would tear its border.
            let modal = Rect::new(area.x, body.y, area.width, footer_y.saturating_sub(body.y));
            let behind = if side_by_side { chat } else { modal };
            f.render_widget(Clear, behind);
            f.render_widget(Block::default().style(style(FG)), behind);
            if !side_by_side {
                self.image_area = Rect::default();
            }
            self.action_rows = Self::draw_popup(
                f,
                popup,
                self.inspection_return.is_some(),
                &self.session.id,
                if side_by_side {
                    Rect::new(chat.x, body.y, chat.width, body.height + composer_h)
                } else {
                    modal
                },
            );
        }
    }
    fn draw_header(&self, f: &mut Frame, r: Rect) {
        let project = self
            .cfg
            .project
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let model = if self.session.demo {
            "offline demo".to_string()
        } else {
            clean(&self.session.model)
        };
        let right = format!("{model} · {}", &self.session.id[..6]);
        let room = (r.width as usize).saturating_sub(right.width() + 4);
        let mut spans = vec![
            Span::styled("✦ aster", style(JADE).add_modifier(Modifier::BOLD)),
            Span::styled("  ", style(FG)),
            Span::styled(tools::clip(&clean(&project), 40), style(FG)),
        ];
        if self.session.mode == "plan" {
            spans.push(Span::styled(
                "  PLAN",
                style(GOLD).add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled("  ·  ", style(LINE)));
        spans.push(Span::styled(
            clean(&self.session.title).replace('\n', " "),
            style(DIM),
        ));
        let left = Line::from(spans);
        let left_width = left.width().min(room.max(20));
        f.render_widget(
            Paragraph::new(left),
            Rect::new(r.x, r.y, left_width as u16, 1),
        );
        if r.width as usize > left_width + right.width() + 4 {
            let len = right.width() as u16;
            f.render_widget(
                Paragraph::new(right).style(style(DIM)),
                Rect::new(r.right().saturating_sub(len), r.y, len, 1),
            );
        }
    }
    fn draw_composer(
        &mut self,
        f: &mut Frame,
        r: Rect,
        columns: usize,
        rows: &[(usize, usize)],
        visible: usize,
    ) {
        let running = self.running.is_some();
        let (mode, mode_color) = if self.session.mode == "plan" {
            ("plan", GOLD)
        } else {
            ("build", JADE)
        };
        let status = if let Some(started) = self.turn_started.filter(|_| running) {
            let ticks = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
            let i = (started.elapsed().as_millis() / 120) as usize % ticks.len();
            Span::styled(
                format!(
                    " {} {} · {}s ",
                    ticks[i],
                    self.state,
                    started.elapsed().as_secs()
                ),
                style(JADE),
            )
        } else if !self.session.pending.is_empty() {
            Span::styled(
                format!(" {} waiting · /queue ", self.session.pending.len()),
                style(GOLD),
            )
        } else {
            Span::styled(" ● ready ", style(DIM))
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style(if self.popup.is_none() { LINE } else { BG }))
            .style(style(FG))
            .title(Line::from(vec![
                Span::styled(format!(" {mode} "), style(mode_color)),
                Span::styled(format!("· {} ", self.cli.permissions), style(DIM)),
            ]))
            .title(Line::from(status).right_aligned());
        f.render_widget(block, r);
        let inner = Rect::new(
            r.x + 2,
            r.y + 1,
            r.width.saturating_sub(4),
            r.height.saturating_sub(2),
        );
        f.render_widget(
            Paragraph::new("›").style(style(JADE)),
            Rect::new(inner.x, inner.y, 1, 1),
        );
        let text_area = Rect::new(inner.x + 2, inner.y, columns as u16, inner.height);
        if self.composer.is_empty() {
            f.render_widget(
                Paragraph::new(if running {
                    "Add a direction · Enter steers · Alt+Enter queues the next task"
                } else {
                    "和弄玉说说，你想做什么？"
                })
                .style(style(DIM)),
                text_area,
            );
            if self.popup.is_none() {
                f.set_cursor_position((text_area.x, text_area.y));
            }
            return;
        }
        let (row, column) = self.composer.cursor_position(columns);
        let start = (row + 1).saturating_sub(visible);
        let lines = rows
            .iter()
            .skip(start)
            .take(visible)
            .map(|&(a, b)| Line::styled(clean(&self.composer.text[a..b]), style(FG)))
            .collect::<Vec<_>>();
        f.render_widget(Paragraph::new(lines), text_area);
        if self.popup.is_none() {
            f.set_cursor_position((
                text_area.x + (column as u16).min(text_area.width.saturating_sub(1)),
                text_area.y + (row - start) as u16,
            ));
        }
        if start > 0 {
            f.render_widget(
                Paragraph::new("↑").style(style(DIM)),
                Rect::new(inner.x, inner.y + 1, 1, 1),
            );
        }
    }
    fn draw_footer(&mut self, f: &mut Frame, r: Rect) {
        let percent = self.context_percent();
        let meter = format!(
            "ctx {percent}%{} · {} in · {} out",
            if self.session.auto_compact && self.cfg.limits.auto_compact > 0 {
                ""
            } else {
                " (auto-compact off)"
            },
            compact_count(self.session.input_tokens),
            compact_count(self.session.output_tokens)
        );
        let meter_color = if percent >= u64::from(self.cfg.limits.auto_compact.max(1)) {
            RED
        } else if percent >= 60 {
            GOLD
        } else {
            DIM
        };
        let fresh = !self.notice.is_empty() && self.notice_at.elapsed() < Duration::from_secs(12);
        let (hint, color) = if fresh {
            (clean(&self.notice), GOLD)
        } else if self.popup.is_some() {
            (String::new(), DIM)
        } else if self.scroll > 0 {
            (
                format!(
                    "↑ reading {} lines above the latest · PgDn · Ctrl+End or Esc returns",
                    self.scroll
                ),
                GOLD,
            )
        } else if !self.session.pending.is_empty() {
            (
                format!(
                    "{} messages waiting · /queue inspect · Esc stops and keeps the queue",
                    self.session.pending.len()
                ),
                DIM,
            )
        } else if self.running.is_some() {
            (
                "Enter steer · Alt+Enter queue · Ctrl+G redirect · F4 output · Esc stop".into(),
                DIM,
            )
        } else if !self.composer.is_empty() {
            (
                "Enter send · Ctrl+J newline · ↑↓ lines & history · Esc Esc clear".into(),
                DIM,
            )
        } else {
            (
                "Enter send · / commands · ↑ history · PgUp scroll · F1 together · Ctrl+P sessions"
                    .into(),
                DIM,
            )
        };
        let meter_width = meter.width() as u16;
        let show_meter = r.width > meter_width + 20;
        let hint_width = if show_meter {
            r.width - meter_width - 2
        } else {
            r.width
        };
        f.render_widget(
            Paragraph::new(fit_hint(&hint, hint_width.saturating_sub(2) as usize))
                .style(style(color)),
            Rect::new(r.x + 1, r.y, hint_width.saturating_sub(1), 1),
        );
        if show_meter {
            f.render_widget(
                Paragraph::new(meter).style(style(meter_color)),
                Rect::new(r.right() - meter_width, r.y, meter_width, 1),
            );
        }
    }
    fn welcome(&self, f: &mut Frame, r: Rect) {
        let texts = [
            ("✦", JADE),
            ("弄玉 · NONGYU", JADE),
            ("", FG),
            ("A little company. A place to make things.", FG),
            ("", FG),
            ("Aster，今天想一起做什么？", FG),
            (
                "Tell me what you have in mind; I'll stay with the work, from idea to check.",
                DIM,
            ),
            ("", FG),
            ("/sessions   pick up where you left off", DIM),
            ("/agents     the rules of this project", DIM),
            ("/demo       try a real file + check", DIM),
            ("↑           recall an earlier request", DIM),
        ];
        let y = r.y + r.height.saturating_sub(texts.len() as u16) / 2;
        let list_width = texts[8..].iter().map(|(t, _)| t.width()).max().unwrap_or(0) as u16;
        let list_x = r.x + r.width.saturating_sub(list_width) / 2;
        for (i, (t, c)) in texts.into_iter().enumerate() {
            let row = y + i as u16;
            if row >= r.bottom() {
                break;
            }
            let paragraph = Paragraph::new(t).style(style(c).add_modifier(if i == 1 {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }));
            if i >= 8 {
                f.render_widget(
                    paragraph,
                    Rect::new(list_x, row, r.right().saturating_sub(list_x), 1),
                );
            } else {
                f.render_widget(
                    paragraph.alignment(ratatui::layout::Alignment::Center),
                    Rect::new(r.x, row, r.width, 1),
                );
            }
        }
    }
    fn conversation(&mut self, f: &mut Frame, r: Rect) {
        let width = r.width.saturating_sub(2) as usize;
        let same_layout = self
            .transcript
            .key
            .as_ref()
            .is_some_and(|(id, _, w, _, _)| id == &self.session.id && *w == width);
        self.transcript
            .update(&self.session, width, self.show_tools);
        let mut tail =
            if self.transcript.after_tool && (!self.stream.is_empty() || self.running.is_some()) {
                vec![line("", FG)]
            } else {
                vec![]
            };
        if !self.stream.is_empty() {
            tail.extend(entry_lines(
                &Entry {
                    role: "nongyu".into(),
                    text: self.stream.clone(),
                },
                width,
                self.show_tools,
            ));
        }
        if self.running.is_some() && self.stream.is_empty() {
            let ticks = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
            let i =
                (chrono::Utc::now().timestamp_millis() / 120).unsigned_abs() as usize % ticks.len();
            let activity = if self.session.work.activity.is_empty() {
                self.state.clone()
            } else {
                self.session.work.activity.to_lowercase()
            };
            tail.push(Line::from(vec![
                Span::styled(format!("{} 弄玉 · {activity}", ticks[i]), style(JADE)),
                Span::styled(
                    format!(
                        " · {}s · Esc stops",
                        self.turn_started.map_or(0, |t| t.elapsed().as_secs())
                    ),
                    style(DIM),
                ),
            ]));
        }
        let total = self.transcript.lines.len() + tail.len();
        if self.scroll > 0 && same_layout {
            self.scroll = if total >= self.transcript.rendered_total {
                self.scroll
                    .saturating_add(total - self.transcript.rendered_total)
            } else {
                self.scroll
                    .saturating_sub(self.transcript.rendered_total - total)
            };
        }
        self.transcript.rendered_total = total;
        let max = total.saturating_sub(r.height as usize);
        self.scroll_max = max;
        self.page = (r.height as usize).saturating_sub(2).max(1);
        if let Some(entry) = self.jump_to.take()
            && let Some(start) = self.transcript.starts.get(entry)
        {
            self.scroll = max.saturating_sub(*start);
        }
        self.scroll = self.scroll.min(max);
        let start = max - self.scroll;
        f.render_widget(
            Paragraph::new(
                self.transcript
                    .lines
                    .iter()
                    .chain(tail.iter())
                    .skip(start)
                    .take(r.height as usize)
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            Rect::new(r.x + 1, r.y, r.width.saturating_sub(1), r.height),
        );
        if self.scroll > 0 && r.height > 2 {
            let label = format!(" ↓ {} newer lines · PgDn · Ctrl+End ", self.scroll);
            let w = (label.width() as u16).min(r.width);
            f.render_widget(
                Paragraph::new(label).style(Style::default().fg(BG).bg(GOLD)),
                Rect::new(r.right().saturating_sub(w), r.bottom() - 1, w, 1),
            );
        }
    }
    fn draw_pet(&mut self, f: &mut Frame, r: Rect) {
        let state = if self.inspection_return.is_some() {
            "reviewing before your decision"
        } else if self.running.is_some() && !self.session.work.waiting.is_empty() {
            "your decision"
        } else if self.running.is_some() {
            self.session.work.activity.as_str()
        } else if matches!(&self.popup,Some(Popup::Project(nav)) if nav.reading()) {
            "reading together"
        } else if matches!(&self.popup, Some(Popup::Project(_))) {
            "finding the right context"
        } else if matches!(self.popup, Some(Popup::History(_))) {
            "looking back together"
        } else if matches!(self.popup, Some(Popup::Tasks { .. })) {
            "choosing the next check"
        } else if self.last_type.elapsed() < Duration::from_secs(2) && !self.composer.is_empty() {
            "listening"
        } else if self.session.work.has_failures() {
            "a check needs attention"
        } else if self.session.work.has_stale_checks() {
            "edits need a fresh check"
        } else {
            "here with you"
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("弄玉", style(FG).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  ◌ {state}"), style(JADE)),
            ])),
            Rect::new(r.x + 1, r.y, r.width.saturating_sub(2), 1),
        );
        let card_height = if r.height >= 26 {
            8
        } else if r.height >= 16 {
            4
        } else {
            1
        };
        let portrait = Rect::new(
            r.x + 1,
            r.y + 2,
            r.width.saturating_sub(2),
            r.height.saturating_sub(3 + card_height + 1),
        );
        let area = fit(portrait, self.portrait_aspect(), self.cell_px);
        self.request_view(portrait);
        self.image_area = area;
        if let Some(frame) = &self.portrait.frame {
            if self.graphics == Graphics::Halfblocks {
                crate::live2d::halfblocks(frame, area, f.buffer_mut());
            }
        } else {
            f.render_widget(
                Paragraph::new(if self.companion.is_some() {
                    clean(&self.portrait.status)
                } else {
                    "Live2D is hidden · /pet on".into()
                })
                .wrap(Wrap { trim: true })
                .style(style(DIM)),
                portrait,
            );
        }
        let divider_y = r.bottom().saturating_sub(card_height + 1);
        f.render_widget(
            Paragraph::new("─".repeat(r.width.saturating_sub(2) as usize)).style(style(LINE)),
            Rect::new(r.x + 1, divider_y, r.width.saturating_sub(2), 1),
        );
        self.work_area = Rect::new(
            r.x,
            r.bottom().saturating_sub(card_height),
            r.width,
            card_height,
        );
        let work = &self.session.work;
        let mut lines = vec![];
        if card_height >= 8 {
            lines.push(line(
                if work.waiting.is_empty() {
                    "WORKING TOGETHER"
                } else {
                    "YOUR DECISION"
                },
                if work.waiting.is_empty() { JADE } else { GOLD },
            ));
            let focus = if !work.waiting.is_empty() {
                work.waiting.clone()
            } else if let Some(step) = work
                .steps
                .iter()
                .find(|s| s.status == crate::work::StepStatus::Doing)
            {
                format!("› {}", step.title)
            } else if !work.focus.is_empty() {
                work.focus.clone()
            } else if !work.goal.is_empty() {
                work.goal.clone()
            } else {
                "Choose a task. I'll track the steps and checks here.".into()
            };
            for text in wrap_prose(&focus, r.width.saturating_sub(2) as usize)
                .into_iter()
                .take(2)
            {
                lines.push(line(text, FG));
            }
            if let Some(skill) = work.skills.last() {
                lines.push(line(format!("Using · {skill}"), JADE));
            } else if !work.context_files.is_empty() {
                lines.push(line(
                    format!("{} attached files · /context", work.context_files.len()),
                    DIM,
                ));
            } else if !work.discovery.is_empty() {
                lines.push(line(work.discovery.lines().next().unwrap_or(""), DIM));
            } else {
                lines.push(line("", DIM));
            }
            if !work.steps.is_empty() {
                let done = work
                    .steps
                    .iter()
                    .filter(|s| s.status == crate::work::StepStatus::Done)
                    .count();
                let bar_width = 10usize;
                let filled = done * bar_width / work.steps.len().max(1);
                lines.push(Line::from(vec![
                    Span::styled("━".repeat(filled), style(JADE)),
                    Span::styled("━".repeat(bar_width - filled), style(LINE)),
                    Span::styled(
                        format!(
                            " {done}/{} steps · {} {}",
                            work.steps.len(),
                            work.changed.len(),
                            if work.changed.len() == 1 {
                                "file"
                            } else {
                                "files"
                            }
                        ),
                        style(DIM),
                    ),
                ]));
            }
        }
        if card_height >= 4 {
            lines.push(line(
                work.verdict(),
                if work.has_failures() || work.has_stale_checks() {
                    GOLD
                } else if work.verified() {
                    JADE
                } else {
                    DIM
                },
            ));
            if let Some(command) = &work.command {
                let tail = if command.stderr_tail.is_empty() {
                    &command.stdout_tail
                } else {
                    &command.stderr_tail
                };
                let last = tail.lines().last().unwrap_or("Waiting for output");
                lines.push(line(
                    format!(
                        "{:.1}s · {}",
                        command.elapsed_ms as f64 / 1000.,
                        clean(last)
                    ),
                    DIM,
                ));
            } else {
                lines.push(line("", DIM));
            }
        }
        lines.push(line(
            fit_hint(
                if work.command.is_some() {
                    "F1 together · F4 output"
                } else {
                    "F1 together · F3 review"
                },
                r.width.saturating_sub(2) as usize,
            ),
            DIM,
        ));
        f.render_widget(
            Paragraph::new(lines),
            Rect::new(
                self.work_area.x + 1,
                self.work_area.y,
                self.work_area.width.saturating_sub(2),
                self.work_area.height,
            ),
        );
    }
    /// Pixel aspect (width / height) of the latest portrait frame.
    fn portrait_aspect(&self) -> f32 {
        420.0 / 620.0 // RENDERER-API: frame.width / frame.height
    }
    /// Ask the renderer for frames that fill this area at the terminal's pixel density.
    fn request_view(&self, _area: Rect) {
        // RENDERER-API: companion.set_view(width_px, height_px)
    }
    fn draw_popup(
        f: &mut Frame,
        p: &mut Popup,
        returning: bool,
        current_session: &str,
        area: Rect,
    ) -> Vec<(Rect, crate::actions::Action)> {
        let width = area.width.saturating_sub(2).min(96);
        let text_height = |text: &str| {
            wrap(text, width.saturating_sub(6) as usize).len() as u16
                + 2
                + if returning { 2 } else { 0 }
        };
        let desired_height = match p {
            Popup::Resources { items, .. } => 9 + 3 * items.len().min(5) as u16,
            Popup::Delete => 7,
            Popup::Redirect { input, .. } => text_height(input) + 6,
            Popup::Question {
                question, options, ..
            } => text_height(question) + options.len() as u16 + 6,
            Popup::Approval(a) => text_height(&a.preview).max(8),
            Popup::Info { text, .. } => text_height(text).max(8),
            _ => 30,
        };
        let height = area.height.saturating_sub(2).min(desired_height);
        let r = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        f.render_widget(Clear, r);
        let hints = match p {
            Popup::Approval(_) => {
                "y allow · n deny · ↑↓ scroll · F2 plan · F6 files · Ctrl+G redirect"
            }
            Popup::Question { .. } => "1–5 choose · Enter send · F6 files · Esc dismiss",
            Popup::Redirect { .. } => "Enter send · Esc return to the decision",
            Popup::Delete => "y delete · n keep · Esc cancel",
            Popup::Info { .. } => "↑↓ PgUp PgDn scroll · Home End · Esc close",
            Popup::Sessions {
                confirm: Some(_), ..
            } => "y delete it · any other key keeps it",
            Popup::Sessions { .. } => {
                "↑↓ choose · Enter open · Ctrl+N new · Ctrl+D delete · Esc close"
            }
            Popup::Resources { .. } => "↑↓ choose · Enter prepare · F1 inspect · Esc close",
            Popup::Tasks { preview: true, .. } => {
                "↑↓ scroll · Tab back to tasks · Enter run · Esc back"
            }
            Popup::Tasks { .. } => "↑↓ choose · Tab inspect · Enter run · Esc close",
            Popup::History(history) if history.preview => "↑↓ scroll · Tab jump · Esc results",
            Popup::History(_) => "↑↓ choose · Enter read · Tab jump · Esc close",
            Popup::Actions(_) => "↑↓ or click · Enter open · Esc back",
            Popup::Project(_) => "",
        };
        let hint_color = if matches!(p, Popup::Approval(_) | Popup::Question { .. }) {
            GOLD
        } else {
            DIM
        };
        let frame = |title: &str| {
            let mut block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(style(if hint_color == GOLD { GOLD } else { LINE }))
                .style(style(FG))
                .title(Line::styled(
                    format!(" {} ", clean(title)),
                    style(JADE).add_modifier(Modifier::BOLD),
                ));
            if !hints.is_empty() {
                block = block.title_bottom(Line::styled(
                    format!(" {} ", fit_hint(hints, width.saturating_sub(6) as usize)),
                    style(hint_color),
                ));
            }
            block
        };
        let inner = r.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let body_height = inner.height.saturating_sub(if returning { 2 } else { 0 });
        let waiting = |f: &mut Frame| {
            if returning {
                f.render_widget(
                    Paragraph::new("Decision still waiting · Esc back · Ctrl+G redirect")
                        .style(style(GOLD)),
                    Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                );
            }
        };
        if let Popup::Actions(menu) = p {
            f.render_widget(frame("Together with 弄玉"), r);
            let filtered = menu.filtered();
            let visible = (body_height.saturating_sub(5) / 2).max(1) as usize;
            let mut rows = vec![];
            let mut write = |text: String, color, y| {
                if y < inner.y + body_height {
                    f.render_widget(
                        Paragraph::new(text).style(style(color)),
                        Rect::new(inner.x, y, inner.width, 1),
                    );
                }
            };
            write("Local controls · no model request".into(), DIM, inner.y);
            write(format!("Find: {}▏", menu.query), FG, inner.y + 1);
            for (row, (index, choice)) in filtered
                .iter()
                .enumerate()
                .skip(menu.index.saturating_sub(visible - 1))
                .take(visible)
                .enumerate()
            {
                let y = inner.y + 3 + row as u16 * 2;
                write(
                    format!(
                        "{} {}",
                        if index == menu.index { "›" } else { " " },
                        choice.label
                    ),
                    if index == menu.index { JADE } else { FG },
                    y,
                );
                write(format!("  {}", choice.detail), DIM, y + 1);
                rows.push((
                    Rect::new(inner.x, y, inner.width, 2).intersection(inner),
                    choice.action,
                ));
            }
            if filtered.is_empty() {
                write(
                    "No matching actions. Ctrl+U clears the filter.".into(),
                    DIM,
                    inner.y + 3,
                );
            }
            write(
                format!(
                    "{}/{}",
                    if filtered.is_empty() {
                        0
                    } else {
                        menu.index + 1
                    },
                    filtered.len()
                ),
                DIM,
                inner.y + body_height.saturating_sub(1),
            );
            waiting(f);
            return rows;
        }
        if let Popup::Sessions {
            items,
            query,
            index,
            confirm,
        } = p
        {
            f.render_widget(frame("Your conversations"), r);
            let filtered = filter_sessions(items, query);
            let mut lines = vec![
                Line::styled(format!("Find: {query}▏"), style(FG)),
                Line::styled(
                    format!(
                        "{} conversation{} in this project",
                        filtered.len(),
                        if filtered.len() == 1 { "" } else { "s" }
                    ),
                    style(DIM),
                ),
                Line::default(),
            ];
            let visible = (body_height.saturating_sub(3) / 3).max(1) as usize;
            let now = chrono::Utc::now();
            for (i, s) in filtered
                .iter()
                .enumerate()
                .skip(index.saturating_sub(visible - 1))
                .take(visible)
            {
                let selected = i == *index;
                let mut title = vec![
                    Span::styled(
                        format!("{} ", if selected { "›" } else { " " }),
                        style(JADE),
                    ),
                    Span::styled(
                        tools::clip(&clean(&s.title).replace('\n', " "), 70)
                            .replace("\n… [truncated]", "…"),
                        style(if selected { JADE } else { FG }).add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                ];
                if s.id == current_session {
                    title.push(Span::styled("  ● current", style(GOLD)));
                }
                if confirm.as_deref() == Some(s.id.as_str()) {
                    title.push(Span::styled("  delete? y / n", style(RED)));
                }
                lines.push(Line::from(title));
                lines.push(Line::styled(
                    format!(
                        "  {} · {} messages · {} · {}",
                        relative_time(&s.updated, now),
                        s.entries.iter().filter(|e| e.role == "you").count(),
                        if s.demo { "demo" } else { s.model.as_str() },
                        &s.id[..6]
                    ),
                    style(DIM),
                ));
                let first = s
                    .entries
                    .iter()
                    .find(|e| e.role == "you")
                    .map(|e| clean(&e.text).replace('\n', " "))
                    .unwrap_or_default();
                lines.push(Line::styled(
                    format!(
                        "  {}",
                        wrap(&first, inner.width.saturating_sub(4) as usize)
                            .first()
                            .cloned()
                            .unwrap_or_default()
                    ),
                    style(DIM),
                ));
            }
            if filtered.is_empty() {
                lines.push(Line::styled("No matching conversations.", style(DIM)));
            }
            f.render_widget(
                Paragraph::new(lines),
                Rect::new(inner.x, inner.y, inner.width, body_height),
            );
            waiting(f);
            return vec![];
        }
        let (title, text, scroll) = match p {
            Popup::Tasks {
                catalog,
                query,
                index,
                preview,
                scroll,
            } => {
                let filtered = catalog
                    .tasks
                    .iter()
                    .filter(|task| {
                        format!("{} {}", task.name, task.description)
                            .to_lowercase()
                            .contains(&query.to_lowercase())
                    })
                    .collect::<Vec<_>>();
                if *preview && let Some(task) = filtered.get(*index) {
                    (
                        format!("Project task · {}", task.name),
                        task.details(),
                        *scroll,
                    )
                } else {
                    let visible = (body_height.saturating_sub(4) / 3).max(1) as usize;
                    let query_line = wrap(&format!("Find: {query}▏"), inner.width as usize)
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    let mut text = format!("{query_line}\nLocal commands · permissions apply\n\n");
                    for (i, task) in filtered
                        .iter()
                        .enumerate()
                        .skip(index.saturating_sub(visible - 1))
                        .take(visible)
                    {
                        text += &format!(
                            "{} {} · {}s\n  {}\n\n",
                            if i == *index { "›" } else { " " },
                            task.name,
                            task.timeout_secs,
                            wrap_prose(&task.description, inner.width.saturating_sub(4) as usize)
                                .first()
                                .cloned()
                                .unwrap_or_default()
                        );
                    }
                    if filtered.is_empty() {
                        text += &format!("No matching tasks.\n{}", catalog.notes.join("\n"));
                    }
                    ("Project tasks beside 弄玉".into(), text, 0)
                }
            }
            Popup::History(history) => {
                if history.preview
                    && let Some(entry) = history.selected()
                {
                    let item = &history.items[entry];
                    let wrapped = wrap_prose(&item.text, inner.width as usize);
                    if history.focus_match {
                        let needle = history.needle();
                        history.scroll = if needle.is_empty() {
                            0
                        } else {
                            wrapped
                                .iter()
                                .position(|line| line.to_lowercase().contains(&needle))
                                .unwrap_or(0)
                                .saturating_sub(2)
                                .min(u16::MAX as usize) as u16
                        };
                        history.focus_match = false;
                    }
                    history.scroll = history.scroll.min(
                        wrapped
                            .len()
                            .saturating_sub(body_height.saturating_sub(1) as usize)
                            .min(u16::MAX as usize) as u16,
                    );
                    (
                        format!("Conversation #{} · {}", entry + 1, item.role),
                        wrapped.join("\n"),
                        history.scroll,
                    )
                } else {
                    let query = wrap(&format!("Find: {}▏", history.query), inner.width as usize)
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    let mut text = format!("{query}\nFilter: you: · nongyu: · tool: · notice:\n\n");
                    let visible = (body_height.saturating_sub(5) / 3).max(1) as usize;
                    for (index, &entry) in history
                        .matches
                        .iter()
                        .enumerate()
                        .skip(history.index.saturating_sub(visible - 1))
                        .take(visible)
                    {
                        let item = &history.items[entry];
                        let excerpt = wrap_prose(
                            &item.text.replace('\n', " "),
                            inner.width.saturating_sub(4) as usize,
                        )
                        .first()
                        .cloned()
                        .unwrap_or_default();
                        text += &format!(
                            "{} #{} · {}\n  {}\n\n",
                            if index == history.index { "›" } else { " " },
                            entry + 1,
                            item.role,
                            excerpt
                        );
                    }
                    if history.matches.is_empty() {
                        text += "No matching conversation entries.\n";
                    }
                    text += &format!(
                        "{} match{} · visible transcript only",
                        history.matches.len(),
                        if history.matches.len() == 1 { "" } else { "es" }
                    );
                    ("History beside 弄玉".into(), text, 0)
                }
            }
            Popup::Actions(_) | Popup::Sessions { .. } => unreachable!("Rendered above"),
            Popup::Project(nav) => nav.view(inner.width as usize, body_height as usize),
            Popup::Resources {
                items,
                skills,
                query,
                index,
            } => {
                let filtered = items
                    .iter()
                    .filter(|r| {
                        format!("{} {}", r.name, r.description)
                            .to_lowercase()
                            .contains(&query.to_lowercase())
                    })
                    .collect::<Vec<_>>();
                let visible = (body_height.saturating_sub(3) / 3).max(1) as usize;
                let mut text = format!("Find: {query}▏\n\n");
                for (i, r) in filtered
                    .iter()
                    .enumerate()
                    .skip(index.saturating_sub(visible - 1))
                    .take(visible)
                {
                    text += &format!(
                        "{} {}{}\n  {}\n\n",
                        if i == *index { "›" } else { " " },
                        r.name,
                        if r.manual_only {
                            " · explicit only"
                        } else {
                            ""
                        },
                        {
                            let lines = wrap_prose(
                                &r.description.replace('\n', " "),
                                inner.width.saturating_sub(4) as usize,
                            );
                            format!(
                                "{}{}",
                                lines.first().cloned().unwrap_or_default(),
                                if lines.len() > 1 { "…" } else { "" }
                            )
                        }
                    );
                }
                if filtered.is_empty() {
                    text += "No matching resources.\n";
                }
                (
                    if *skills {
                        "Skills beside 弄玉"
                    } else {
                        "Reusable prompts"
                    }
                    .into(),
                    text,
                    0,
                )
            }
            Popup::Redirect { input, .. } => (
                "弄玉 · change direction".into(),
                format!(
                    "Tell me what to change.\nPending actions will be cancelled when you send.\n\n› {input}▏"
                ),
                0,
            ),
            Popup::Question {
                question,
                options,
                input,
                ..
            } => (
                "弄玉 · a question for you".into(),
                format!(
                    "{}\n\n{}\n\nOr type an answer:\n› {}▏",
                    question,
                    options
                        .iter()
                        .enumerate()
                        .map(|(i, o)| format!("[{}] {o}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    input
                ),
                0,
            ),
            Popup::Info {
                title,
                text,
                scroll,
            } => {
                let lines = wrap(text, inner.width as usize).len();
                *scroll = (*scroll).min(
                    lines
                        .saturating_sub(body_height as usize)
                        .min(u16::MAX as usize) as u16,
                );
                (title.clone(), text.clone(), *scroll)
            }
            Popup::Approval(a) => {
                let lines = wrap(&a.preview, inner.width as usize).len();
                a.scroll = a.scroll.min(
                    lines
                        .saturating_sub(body_height as usize)
                        .min(u16::MAX as usize) as u16,
                );
                (format!("Allow {}?", a.tool), a.preview.clone(), a.scroll)
            }
            Popup::Delete => (
                "Delete this conversation?".into(),
                "The saved conversation will be removed.\nProject files stay in place.".into(),
                0,
            ),
        };
        f.render_widget(frame(&title), r);
        let diff_panel = matches!(p, Popup::Approval(a) if matches!(a.tool.as_str(), "write_file" | "edit_file" | "multi_edit" | "delete_file" | "move_file"))
            || matches!(p, Popup::Info{title,..} if title.starts_with("Review changes"));
        let body = if diff_panel {
            ratatui::text::Text::from(crate::richtext::diff(&text, inner.width as usize))
        } else {
            ratatui::text::Text::from(clean(&text))
        };
        f.render_widget(
            Paragraph::new(body)
                .style(style(FG))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            Rect::new(inner.x, inner.y, inner.width, body_height),
        );
        waiting(f);
        vec![]
    }
}
fn filter_sessions<'a>(items: &'a [Session], query: &str) -> Vec<&'a Session> {
    let query = query.to_lowercase();
    items
        .iter()
        .filter(|s| {
            query.is_empty()
                || s.title.to_lowercase().contains(&query)
                || s.id.contains(&query)
                || s.entries
                    .iter()
                    .filter(|e| e.role == "you")
                    .take(3)
                    .any(|e| e.text.to_lowercase().contains(&query))
        })
        .collect()
}
fn relative_time(stamp: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(stamp) else {
        return "earlier".into();
    };
    let seconds = (now - then.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0);
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        86_400..604_800 => format!("{} d ago", seconds / 86_400),
        _ => then.format("%Y-%m-%d").to_string(),
    }
}
/// Drop whole " · " separated hints from the end until the line fits.
fn fit_hint(hint: &str, width: usize) -> String {
    let mut parts = hint.split(" · ").collect::<Vec<_>>();
    while parts.len() > 1 && parts.join(" · ").width() > width {
        parts.pop();
    }
    let text = parts.join(" · ");
    if text.width() <= width {
        return text;
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) + 1 > width {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}
fn compact_count(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{:.1}k", n as f64 / 1000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}
/// Terminal cell size in pixels; a common 1:2 cell when the terminal does not say.
fn cell_pixels() -> (f32, f32) {
    crossterm::terminal::window_size()
        .ok()
        .filter(|w| w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0)
        .map(|w| {
            (
                f32::from(w.width) / f32::from(w.columns),
                f32::from(w.height) / f32::from(w.rows),
            )
        })
        .filter(|(w, h)| (2.0..=64.0).contains(w) && (4.0..=128.0).contains(h))
        .unwrap_or((9.0, 18.0))
}
/// The largest area inside `area` with the given pixel aspect, centered horizontally.
fn fit(area: Rect, aspect: f32, cell: (f32, f32)) -> Rect {
    if area.width == 0 || area.height == 0 || aspect <= 0.0 {
        return area;
    }
    let width_px = f32::from(area.width) * cell.0;
    let height_px = f32::from(area.height) * cell.1;
    let (width, height) = if width_px / height_px > aspect {
        (
            ((height_px * aspect / cell.0).round() as u16).clamp(1, area.width),
            area.height,
        )
    } else {
        (
            area.width,
            ((width_px / aspect / cell.1).round() as u16).clamp(1, area.height),
        )
    };
    Rect::new(area.x + (area.width - width) / 2, area.y, width, height)
}
/// Earlier requests from this project's recent conversations, oldest first.
fn recent_prompts(store: &Store, current: &Session) -> Vec<String> {
    let mut sessions = store.list(&current.project).unwrap_or_default();
    sessions.retain(|s| s.id != current.id);
    sessions.truncate(8);
    sessions.reverse();
    sessions.push(current.clone());
    sessions
        .iter()
        .flat_map(|s| s.entries.iter())
        .filter(|e| e.role == "you" && !e.text.starts_with("Answer: "))
        .map(|e| e.text.clone())
        .collect()
}
pub fn run(cfg: Config, cli: Cli, store: Store) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Use a terminal for the workbench, or --prompt for a single command-line turn")
    }
    let mut app = App::new(cfg, cli, store)?;
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
        while !app.quit {
            if crate::lifecycle::requested() {
                app.request_quit();
            }
            app.tick()?;
            if app.quit {
                break;
            }
            if crate::lifecycle::requested() {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let old_area = app.last_image.map(|(_, r)| r);
            terminal.draw(|f| app.draw(f))?;
            if old_area.is_some_and(|r| r != app.image_area) {
                write!(io::stdout(), "{}", app.graphics.clear())?;
                // Fullscreen redraw needs no cursor-position round trip. A queued
                // Escape or an emulator without a reply must not terminate Aster.
                terminal.resize(terminal.size()?.into())?;
                terminal.draw(|f| app.draw(f))?;
                app.last_image = None;
            }
            if app.image_area.width > 0
                && matches!(app.graphics, Graphics::Iterm | Graphics::Kitty)
                && let Some(frame) = &app.portrait.frame
                && app.last_image != Some((frame.sequence, app.image_area))
            {
                write!(
                    io::stdout(),
                    "{}",
                    app.graphics.encode(&frame.png, app.image_area)
                )?;
                io::stdout().flush()?;
                app.last_image = Some((frame.sequence, app.image_area));
            }
            if event::poll(Duration::from_millis(33))? {
                match event::read()? {
                    TermEvent::Key(key) => {
                        if let Err(e) = app.key(key) {
                            app.notify(e.to_string())
                        }
                    }
                    TermEvent::Paste(text) => {
                        app.paste(&text);
                    }
                    TermEvent::Resize(_, _) => {
                        app.cell_px = cell_pixels();
                        write!(io::stdout(), "{}", app.graphics.clear())?;
                        terminal.resize(terminal.size()?.into())?;
                        app.last_image = None;
                    }
                    TermEvent::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            let up = matches!(mouse.kind, MouseEventKind::ScrollUp);
                            if app.popup.is_some() {
                                // Reading panels scroll like the transcript; lists move one row.
                                let steps = if matches!(
                                    app.popup,
                                    Some(Popup::Info { .. } | Popup::Approval(_))
                                ) {
                                    3
                                } else {
                                    1
                                };
                                for _ in 0..steps {
                                    if let Err(e) = app.key(KeyEvent::new(
                                        if up { KeyCode::Up } else { KeyCode::Down },
                                        KeyModifiers::NONE,
                                    )) {
                                        app.notify(e.to_string());
                                    }
                                }
                            } else {
                                app.scroll_by(if up { 3 } else { -3 });
                            }
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            if let Err(e) = app.click(mouse.column, mouse.row) {
                                app.notify(e.to_string());
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        app.persist()?;
        Ok(())
    })();
    if result.is_err() {
        app.request_quit();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.running.is_some() && Instant::now() < deadline {
            if app.tick().is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if app.running.is_some() {
            app.session.status = "interrupted".into();
        }
        let _ = app.persist();
    }
    let _ = write!(io::stdout(), "{}", app.graphics.clear());
    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
    result
}
pub fn headless(cfg: Config, cli: Cli, store: Store) -> Result<()> {
    let mut s = if let Some(id) = &cli.resume {
        store.load(id)?
    } else if cli.continue_last {
        store
            .list(&cfg.project)?
            .into_iter()
            .next()
            .unwrap_or_else(|| Session::new(cfg.project.clone(), cfg.model.clone(), cli.demo))
    } else {
        Session::new(cfg.project.clone(), cfg.model.clone(), cli.demo)
    };
    if s.project != cfg.project {
        bail!("Session belongs to another project")
    }
    let prompt = cli.prompt.as_deref().context("No prompt")?;
    let prior = s.clone();
    s.add("you", prompt);
    s.messages.push(json!({"role":"user","content":prompt}));
    s.status = "thinking".into();
    store.save(&s)?;
    let running = agent::spawn(prior, prompt.into(), cfg, cli.permissions);
    let mut stopping = None;
    loop {
        if crate::lifecycle::requested() {
            running.cancel.store(true, Ordering::Relaxed);
            let started = stopping.get_or_insert_with(Instant::now);
            if started.elapsed() > Duration::from_secs(2) {
                s.status = "interrupted".into();
                s.add(
                    "notice",
                    "Interrupted during shutdown. Inspect project state before continuing.",
                );
                store.save(&s)?;
                bail!("Interrupted by shutdown signal");
            }
        }
        let event = match running.events.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => event,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        match event {
            Event::Delta(t) => {
                print!("{}", clean(&t));
                io::stdout().flush()?;
            }
            Event::Entry(role, text) if role != "nongyu" => println!("\n[{role}] {}", clean(&text)),
            Event::Approval { answer, .. } => {
                let _ = answer.send(false);
                eprintln!(
                    "Action declined: headless mode cannot ask. Use --permissions allow for explicitly authorized actions."
                );
            }
            Event::Question { answer, .. } => {
                let _ = answer.send(String::new());
                eprintln!("Question unanswered: interactive input is needed.");
            }
            Event::Work(work) => s.work = *work,
            Event::Usage(input, output) => {
                s.input_tokens = input;
                s.output_tokens = output;
            }
            Event::Checkpoint(checkpoint) => {
                s = *checkpoint;
                store.save(&s)?;
            }
            Event::Finished(s) => {
                store.save(&s)?;
                println!(
                    "\nSession {} · {} · {} input / {} output tokens",
                    s.id, s.status, s.input_tokens, s.output_tokens
                );
                if matches!(s.status.as_str(), "error" | "stopped" | "interrupted") {
                    bail!("Turn ended with an error")
                };
                if cli
                    .prompt
                    .as_ref()
                    .is_some_and(|p| p.starts_with("/run ") || p.starts_with("/task "))
                    && !s
                        .work
                        .command
                        .as_ref()
                        .is_some_and(|c| c.exit_code == Some(0) && !c.stopped && !c.timed_out)
                {
                    bail!("Local command did not succeed; inspect its saved output");
                }
                if s.work.has_failures() || s.work.has_stale_checks() {
                    bail!(
                        "{}; inspect /checks in the saved conversation",
                        s.work.verdict()
                    );
                }
                return Ok(());
            }
            _ => {}
        }
    }
    bail!("Agent worker stopped unexpectedly")
}
pub fn screenshot(cfg: Config, cli: Cli, store: Store, path: &Path) -> Result<()> {
    let mut app = App::new(cfg, cli.clone(), store)?;
    if let Some(panel) = &cli.preview_panel {
        app.command(&format!("/{panel}"))?;
    }
    let start = Instant::now();
    loop {
        if crate::lifecycle::requested() {
            bail!("Preview interrupted");
        }
        app.tick()?;
        let portrait_ready = app.companion.is_none()
            || app.portrait.frame.is_some()
            || app.portrait.status.starts_with("Live2D unavailable");
        let panel_ready = !matches!(&app.popup,Some(Popup::Project(nav)) if nav.busy());
        if portrait_ready && panel_ready {
            break;
        }
        if start.elapsed() > Duration::from_secs(70) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    app.graphics = Graphics::Iterm;
    let mut terminal = Terminal::new(TestBackend::new(cli.width, cli.height))?;
    terminal.draw(|f| app.draw(f))?;
    let buf = terminal.backend().buffer();
    let cw = 9;
    let ch = 18;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#111318\"/><g font-family=\"Menlo,monospace\" font-size=\"13\">",
        cli.width as usize * cw,
        cli.height as usize * ch,
        cli.width as usize * cw,
        cli.height as usize * ch
    );
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    for y in 0..cli.height {
        for x in 0..cli.width {
            let c = &buf[(x, y)];
            if c.symbol().trim().is_empty() {
                continue;
            }
            let color = match c.fg {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                _ => "#dedad0".into(),
            };
            svg += &format!(
                "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-weight=\"{}\" font-style=\"{}\" text-decoration=\"{}\">{}</text>",
                x as usize * cw,
                y as usize * ch + 14,
                color,
                if c.modifier.contains(Modifier::BOLD) {
                    "bold"
                } else {
                    "normal"
                },
                if c.modifier.contains(Modifier::ITALIC) {
                    "italic"
                } else {
                    "normal"
                },
                match (
                    c.modifier.contains(Modifier::UNDERLINED),
                    c.modifier.contains(Modifier::CROSSED_OUT)
                ) {
                    (true, true) => "underline line-through",
                    (true, false) => "underline",
                    (false, true) => "line-through",
                    _ => "none",
                },
                esc(c.symbol())
            );
        }
    }
    svg += "</g>";
    if let Some(frame) = &app.portrait.frame {
        use base64::Engine;
        let r = app.image_area;
        svg += &format!(
            "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"xMidYMid meet\" href=\"data:image/png;base64,{}\"/>",
            r.x as usize * cw,
            r.y as usize * ch,
            r.width as usize * cw,
            r.height as usize * ch,
            base64::engine::general_purpose::STANDARD.encode(&frame.png)
        );
    }
    svg += "</svg>";
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?
    }
    fs::write(path, svg)?;
    println!("Rendered workbench preview: {}", path.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prose_wrap_preserves_words_and_code_indentation() {
        assert_eq!(
            wrap_prose("A focused edit preserves the file.", 15),
            vec!["A focused edit", "preserves the", "file."]
        );
        assert!(
            wrap_prose("你好，Aster。一起检查结果。", 12)
                .iter()
                .all(|s| s.width() <= 12)
        );
        assert_eq!(
            wrap_prose("```rust\n    let x = 2;\n```", 30),
            vec!["```rust", "    let x = 2;", "```"]
        );
    }
    #[test]
    fn unicode_wrap_and_escape_sanitization() {
        assert_eq!(wrap("你好世界", 4), vec!["你好", "世界"]);
        assert!(!clean("bad\x1b[31m").contains('\x1b'));
        assert_eq!(wrap("a\nb", 20), vec!["a", "b"]);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    fn app(root: &Path) -> App {
        let cli = Cli::parse_from(["aster", "--no-live2d", "--demo"]);
        let cfg = Config {
            home: root.into(),
            project: root.into(),
            state: root.join("state"),
            key: String::new(),
            base: "https://api.minimaxi.com/anthropic".into(),
            model: "MiniMax-M2.7".into(),
            pet: root.join("pet"),
            chrome: root.join("chrome"),
            companion_profile: None,
            texture_size: 2048,
            limits: Default::default(),
        };
        let store = Store::open(&cfg.state).unwrap();
        App::new(cfg, cli, store).unwrap()
    }
    use clap::Parser;
    fn finish(a: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(12);
        while a.running.is_some() {
            assert!(Instant::now() < deadline);
            a.tick().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn stopped_follow_up_survives_resume_without_autorunning() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.submit("first message".into()).unwrap();
        a.enqueue("saved next task".into(), Delivery::FollowUp)
            .unwrap();
        a.stop();
        finish(&mut a);
        let id = a.session.id.clone();
        assert_eq!(a.store.load(&id).unwrap().pending.len(), 1);
        let cfg = a.cfg.clone();
        let mut cli = a.cli.clone();
        cli.resume = Some(id);
        drop(a);
        let store = Store::open(&cfg.state).unwrap();
        let mut b = App::new(cfg, cli, store).unwrap();
        assert!(b.running.is_none());
        assert_eq!(b.session.pending[0].text, "saved next task");
        b.command("/next").unwrap();
        finish(&mut b);
        assert!(b.session.pending.is_empty());
        assert!(
            b.session
                .entries
                .iter()
                .any(|e| e.role == "you" && e.text == "saved next task")
        );
    }
    #[test]
    fn follow_up_runs_once_after_a_normal_turn() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.submit("first message".into()).unwrap();
        a.enqueue("second message".into(), Delivery::FollowUp)
            .unwrap();
        finish(&mut a);
        assert!(a.session.pending.is_empty());
        assert_eq!(
            a.session
                .entries
                .iter()
                .filter(|e| e.role == "you" && e.text == "second message")
                .count(),
            1
        );
        assert!(a.store.load(&a.session.id).unwrap().pending.is_empty());
    }
    #[test]
    fn manual_checks_refresh_exact_evidence_and_preserve_provider_pairs() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        fs::write(a.cfg.project.join("answer.json"), "{\"ok\":false}").unwrap();
        a.command("/check answer.json {\"ok\":true}").unwrap();
        assert!(a.session.work.has_failures());
        a.command("/check answer.json").unwrap();
        assert!(a.session.work.has_failures());
        fs::write(a.cfg.project.join("answer.json"), "{\"ok\":true}").unwrap();
        a.command("/check answer.json {\"ok\":true}").unwrap();
        assert!(a.session.work.verified());
        assert_eq!(a.reaction.as_ref().unwrap().0, "pleased");
        assert_eq!(a.notice, "Recorded checks passed");
        assert_eq!(a.session.work.model_requests, 0);
        let messages = &a.session.messages;
        let assistant = &messages[messages.len() - 2]["content"][0];
        let result = &messages[messages.len() - 1]["content"][0];
        assert_eq!(assistant["id"], result["tool_use_id"]);
        fs::write(a.cfg.project.join("answer.json"), "invalid JSON").unwrap();
        a.command("/check answer.json {\"ok\":true}").unwrap();
        assert!(a.session.work.has_failures());
        assert_eq!(a.reaction.as_ref().unwrap().0, "concerned");
        a.command("/checks").unwrap();
        assert!(
            matches!(&a.popup, Some(Popup::Info { title, text, .. }) if title.starts_with("Checks beside") && text.contains("earlier result"))
        );
    }
    #[test]
    fn history_preview_opens_near_the_match_and_home_reads_the_beginning() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        let mut text = (0..300)
            .map(|i| format!("original line {i}\n"))
            .collect::<String>();
        text.push_str("needle-history-deep\nlast line\n");
        a.session.add("nongyu", text);
        a.command("/history needle-history-deep").unwrap();
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(132, 42)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("needle-history-deep")
        );
        a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let visible = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(visible.contains("original line 0"));
        assert!(!visible.contains("needle-history-deep"));
    }
    #[test]
    fn transcript_layout_reuses_idle_frames_and_invalidates_after_changes() {
        let mut session = Session::new("/project".into(), "test".into(), true);
        session.add(
            "you",
            "A long line with Unicode 中文 and enough words to wrap.",
        );
        session.add("tool", "read_file source.txt\nVisible tool output");
        let mut layout = TranscriptLayout::default();
        assert!(layout.update(&session, 30, false));
        let initial = layout.lines.len();
        for _ in 0..100 {
            assert!(!layout.update(&session, 30, false));
        }
        assert!(layout.update(&session, 30, true));
        assert!(layout.lines.len() > initial);
        assert!(layout.update(&session, 60, true));
        session.add("nongyu", "A new reply.");
        assert!(layout.update(&session, 60, true));
        assert_eq!(layout.starts.len(), 3);
    }
    #[test]
    fn history_jump_keeps_the_draft_and_stays_put_when_new_entries_arrive() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        for index in 0..100 {
            a.session.add(
                "you",
                format!("Earlier request number {index} with its original details."),
            );
        }
        a.input_set("Keep my next request");
        a.command("/history number 20 ").unwrap();
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(&a.popup,Some(Popup::History(history)) if history.preview));
        a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(132, 42)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        fn rows(buffer: &ratatui::buffer::Buffer, range: std::ops::Range<u16>) -> Vec<String> {
            range
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect()
                })
                .collect()
        }
        let before = rows(terminal.backend().buffer(), 0..30);
        assert!(before.concat().contains("Earlier request number 20"));
        a.session.add(
            "nongyu",
            "A later message that should not move the history viewport.",
        );
        terminal.draw(|f| a.draw(f)).unwrap();
        // The transcript viewport stays put; only the "newer lines" count changes.
        assert_eq!(rows(terminal.backend().buffer(), 0..30), before);
        a.key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL))
            .unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("A later message that should not move")
        );
        assert_eq!(a.composer.text, "Keep my next request");
        assert!(a.session.messages.is_empty());
    }
    #[test]
    fn checkpoint_archive_failure_leaves_the_active_and_saved_context_intact() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.session.add("you", "Keep my original request.");
        a.session
            .messages
            .push(json!({"role":"user","content":"history".repeat(20_000)}));
        a.persist().unwrap();
        let before = a.session.messages.clone();
        fs::write(a.store.root.join("archive"), "blocked directory fixture").unwrap();
        assert!(a.compact("").is_err());
        assert_eq!(a.session.messages, before);
        assert_eq!(a.store.load(&a.session.id).unwrap().messages, before);
        assert!(a.session.checkpoint.is_none());
    }
    #[test]
    fn resource_selection_and_pasted_filters_keep_the_existing_request() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        let skill = a.cfg.project.join(".aster/skills/review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Review a change\n---\nRead before editing.",
        )
        .unwrap();
        a.input_set("Explain this change 中文");
        a.show_resources(true);
        a.paste("review");
        assert!(matches!(&a.popup, Some(Popup::Resources{query,..}) if query=="review"));
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(a.composer.text, "/skill review Explain this change 中文");
        assert!(a.session.messages.is_empty());
        a.command("/sessions").unwrap();
        a.paste("fresh");
        assert!(matches!(&a.popup, Some(Popup::Sessions{query,..}) if query=="fresh"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(a.composer.text, "/skill review Explain this change 中文");
    }
    #[test]
    fn companion_actions_support_mouse_and_preserve_pending_decisions() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.input_set("Keep this composer draft");
        let (answer, rx) = crossbeam_channel::bounded(1);
        a.popup = Some(Popup::Approval(Approval {
            tool: "write_file".into(),
            preview: "Pending write".into(),
            scroll: 0,
            answer,
        }));
        a.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .unwrap();
        a.paste("files");
        let mut terminal = Terminal::new(TestBackend::new(132, 42)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let (row, _) = a
            .action_rows
            .iter()
            .find(|(_, action)| *action == crate::actions::Action::Command("/files"))
            .unwrap();
        a.click(row.x + 1, row.y).unwrap();
        assert!(matches!(a.popup, Some(Popup::Project(_))));
        assert!(matches!(
            rx.try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        ));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(a.popup, Some(Popup::Approval(_))));
        a.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .unwrap();
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(a.popup, Some(Popup::Approval(_))));
        assert_eq!(a.composer.text, "Keep this composer draft");
        assert!(a.session.messages.is_empty());
        assert_eq!(a.session.work.model_requests, 0);
    }
    #[test]
    fn inspecting_keeps_approval_and_question_channels_open() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.input_set("Keep my draft");
        let (answer, rx) = crossbeam_channel::bounded(1);
        a.popup = Some(Popup::Approval(Approval {
            tool: "edit_file".into(),
            preview: "original diff".into(),
            scroll: 5,
            answer,
        }));
        for function in [2, 3, 4, 5, 6, 2] {
            a.key(KeyEvent::new(KeyCode::F(function), KeyModifiers::NONE))
                .unwrap();
            assert!(matches!(
                rx.try_recv(),
                Err(crossbeam_channel::TryRecvError::Empty)
            ));
            assert!(matches!(
                a.inspection_return.as_deref(),
                Some(Popup::Approval(_))
            ));
        }
        // Approval keys belong only to the visible approval, never an inspection.
        a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        assert!(rx.try_recv().is_err());
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(
            matches!(&a.popup,Some(Popup::Approval(v)) if v.scroll == 5 && v.preview == "original diff")
        );
        a.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .unwrap();
        assert!(!rx.recv().unwrap());
        assert!(a.popup.is_none());
        let (answer, rx) = crossbeam_channel::bounded(1);
        a.popup = Some(Popup::Question {
            question: "Language?".into(),
            options: vec![],
            input: "中".into(),
            answer,
        });
        a.command("/checks").unwrap();
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(rx.try_recv().is_err());
        a.paste("文");
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(rx.recv().unwrap(), "中文");
        assert_eq!(a.composer.text, "Keep my draft");
    }
    #[test]
    fn closed_decisions_cannot_return_from_an_inspection_or_redirect() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        let (answer, rx) = crossbeam_channel::bounded(1);
        a.popup = Some(Popup::Redirect {
            input: "New direction".into(),
            previous: Some(Box::new(Popup::Approval(Approval {
                tool: "write_file".into(),
                preview: "obsolete diff".into(),
                scroll: 0,
                answer,
            }))),
        });
        a.show_work();
        a.clear_decision();
        assert!(matches!(
            rx.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        ));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(
            matches!(&a.popup,Some(Popup::Redirect{input,previous}) if input=="New direction" && previous.is_none())
        );
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(a.popup.is_none());
        assert!(a.inspection_return.is_none());
    }
    #[test]
    fn cancelling_while_inspecting_drops_the_pending_write() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.submit("steering demo".into()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !matches!(a.popup, Some(Popup::Approval(_))) {
            assert!(Instant::now() < deadline);
            a.tick().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        a.show_work();
        a.stop();
        finish(&mut a);
        assert_eq!(a.session.status, "stopped");
        assert!(a.inspection_return.is_none());
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(a.popup.is_none());
        assert!(!d.path().join("stale.json").exists());
    }
    #[test]
    fn project_picker_attaches_to_the_draft_without_submitting() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        fs::write(a.cfg.project.join("notes.txt"), "jade context\n").unwrap();
        a.input_set("Explain this");
        a.command("/files notes").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while matches!(&a.popup,Some(Popup::Project(nav)) if nav.busy()) {
            assert!(Instant::now() < deadline);
            a.tick().unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(a.composer.text, "Explain this @{notes.txt:1-80} ");
        assert!(a.popup.is_none());
        assert!(a.running.is_none());
        assert!(a.session.messages.is_empty());
        assert_eq!(a.session.work.model_requests, 0);
    }
    #[test]
    fn task_inspection_keeps_selection_draft_and_pending_decision() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".aster")).unwrap();
        std::fs::write(
            d.path().join(".aster/tasks.json"),
            r#"{"tasks":[{"name":"proof","command":"touch proof"}]}"#,
        )
        .unwrap();
        let mut a = app(d.path());
        a.input_set("Keep my draft");
        let (answer, rx) = crossbeam_channel::bounded(1);
        a.popup = Some(Popup::Question {
            question: "Choose".into(),
            options: vec![],
            input: "Partial answer".into(),
            answer,
        });
        a.command("/tasks").unwrap();
        a.paste("proof");
        a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        a.paste("another task");
        assert!(matches!(&a.popup, Some(Popup::Tasks{query,preview:true,..}) if query == "proof"));
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(a.running.is_none());
        assert!(!d.path().join("proof").exists());
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(&a.popup, Some(Popup::Question{input,..}) if input == "Partial answer"));
        assert!(rx.try_recv().is_err());
        assert_eq!(a.composer.text, "Keep my draft");
        assert_eq!(a.session.work.model_requests, 0);
    }
    #[test]
    fn pasted_question_answer_keeps_the_composer_draft() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.input_set("Keep my draft");
        let (answer, rx) = crossbeam_channel::bounded(1);
        a.popup = Some(Popup::Question {
            question: "Which language?".into(),
            options: vec![],
            input: String::new(),
            answer,
        });
        a.paste("中文");
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(rx.recv().unwrap(), "中文");
        assert_eq!(a.composer.text, "Keep my draft");
    }
    #[test]
    fn redirect_can_return_to_decision_or_cancel_it_with_new_input() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.submit("steering demo".into()).unwrap();
        a.input_set("Keep this draft");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !matches!(a.popup, Some(Popup::Approval(_))) {
            assert!(Instant::now() < deadline);
            a.tick().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        let redirect = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        a.command("/files").unwrap();
        a.key(redirect).unwrap();
        a.show_work();
        // Ctrl+G restores an existing redirect draft, without nesting decisions.
        a.key(redirect).unwrap();
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(a.popup, Some(Popup::Approval(_))));
        assert!(!d.path().join("stale.json").exists());
        a.key(redirect).unwrap();
        a.paste("Use steer-proof-486 instead.");
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        while a.running.is_some() {
            assert!(Instant::now() < deadline);
            a.tick().unwrap();
            if let Some(Popup::Approval(approval)) = &a.popup {
                assert!(approval.preview.contains("steered.json"));
                a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
                    .unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(a.composer.text, "Keep this draft");
        assert!(!d.path().join("stale.json").exists());
        assert!(d.path().join("steered.json").exists());
        assert!(a.session.pending.is_empty());
    }
    #[test]
    fn renders_compact_wide_and_popups() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        for (w, h) in [(45, 12), (80, 24), (132, 42), (200, 60)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| a.draw(f)).unwrap();
            a.input_set("/s");
            terminal.draw(|f| a.draw(f)).unwrap();
            a.input_set("");
            a.command("/help").unwrap();
            terminal.draw(|f| a.draw(f)).unwrap();
            a.popup = None;
            a.show_actions();
            terminal.draw(|f| a.draw(f)).unwrap();
            a.paste("commands do not match a very long query with repeated text to wrap around the terminal");
            terminal.draw(|f| a.draw(f)).unwrap();
            a.popup = None;
        }
    }
    #[test]
    fn unicode_composer_and_named_fork() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        for c in "你好Aster".chars() {
            a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
                .unwrap();
        }
        a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
            .unwrap();
        a.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(a.composer.text, "好Aster");
        a.command("/rename Test parent").unwrap();
        let parent = a.session.id.clone();
        a.command("/fork Test child").unwrap();
        assert_eq!(a.session.parent, Some(parent.clone()));
        assert_eq!(a.store.load(&parent).unwrap().title, "Test parent");
        assert_eq!(a.session.title, "Test child");
    }
    fn screen(a: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn press(a: &mut App, code: KeyCode) {
        a.key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }
    #[test]
    fn arrow_keys_recall_prompts_and_never_strand_the_transcript() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.prompts.push("first request");
        a.prompts.push("second request");
        a.input_set("unsent draft");
        // Old behavior: ↑ silently scrolled a short transcript and pinned the view.
        press(&mut a, KeyCode::Up);
        assert_eq!(a.composer.text, "second request");
        press(&mut a, KeyCode::Up);
        assert_eq!(a.composer.text, "first request");
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Down);
        assert_eq!(a.composer.text, "unsent draft");
        for _ in 0..20 {
            press(&mut a, KeyCode::Up);
            press(&mut a, KeyCode::PageUp);
        }
        screen(&mut a, 100, 30);
        assert_eq!(a.scroll, 0, "nothing to scroll in an empty conversation");
        // History stops at the oldest request, and ↓ walks back to the draft.
        assert_eq!(a.composer.text, "first request");
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Down);
        assert_eq!(a.composer.text, "unsent draft");
        for n in 0..60 {
            a.session.add("nongyu", format!("reply number {n}"));
        }
        let visible = screen(&mut a, 100, 30);
        assert!(visible.contains("reply number 59"), "{visible}");
        press(&mut a, KeyCode::PageUp);
        let scrolled = screen(&mut a, 100, 30);
        assert!(a.scroll > 0 && !scrolled.contains("reply number 59"));
        assert!(scrolled.contains("newer lines"));
        for _ in 0..500 {
            press(&mut a, KeyCode::PageUp);
        }
        assert_eq!(a.scroll, a.scroll_max, "scrolling stops at the first line");
        screen(&mut a, 100, 30);
        press(&mut a, KeyCode::PageDown);
        assert!(
            a.scroll < a.scroll_max,
            "one PageDown moves back immediately"
        );
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.scroll, 0);
        assert_eq!(a.composer.text, "unsent draft");
        assert!(screen(&mut a, 100, 30).contains("reply number 59"));
    }
    #[test]
    fn escape_and_ctrl_c_clear_drafts_recoverably_and_option_arrows_type_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.input_set("abc");
        a.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT))
            .unwrap();
        a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
            .unwrap();
        assert_eq!(a.composer.text, "abc");
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.composer.text, "abc", "one Esc only warns");
        press(&mut a, KeyCode::Esc);
        assert!(a.composer.is_empty());
        press(&mut a, KeyCode::Up);
        assert_eq!(a.composer.text, "abc");
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(a.composer.is_empty() && !a.quit);
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(a.quit);
        // Esc closes the command palette it opened.
        drop(a);
        let mut a = app(d.path());
        a.input_set("/");
        assert!(!a.suggestions().is_empty());
        press(&mut a, KeyCode::Esc);
        assert!(a.composer.is_empty());
    }
    #[test]
    fn session_picker_opens_another_conversation_and_confirms_deletes() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.command("/rename Older work").unwrap();
        a.session.add("you", "Please fix the parser");
        a.persist().unwrap();
        let older = a.session.id.clone();
        a.command("/new Scratch").unwrap();
        let scratch = a.session.id.clone();
        a.command("/new Current").unwrap();
        let current = a.session.id.clone();
        a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
            .unwrap();
        let text = screen(&mut a, 132, 42);
        assert!(text.contains("Your conversations") && text.contains("● current"));
        // The default selection is the most recent other conversation.
        assert!(
            matches!(&a.popup, Some(Popup::Sessions{items,index,..}) if items[*index].id != current)
        );
        a.paste("parser");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.session.id, older);
        assert!(a.popup.is_none());
        assert!(screen(&mut a, 132, 42).contains("Please fix the parser"));
        a.command("/sessions").unwrap();
        a.paste("Scratch");
        a.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(screen(&mut a, 132, 42).contains("delete? y / n"));
        press(&mut a, KeyCode::Char('y'));
        assert!(a.store.load(&scratch).is_err());
        assert!(a.store.load(&current).is_ok());
        assert!(matches!(a.popup, Some(Popup::Sessions { .. })));
        press(&mut a, KeyCode::Esc);
        assert!(a.popup.is_none());
        assert_eq!(a.session.id, older);
    }
    #[test]
    fn background_compaction_summarizes_without_blocking_the_terminal() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        for n in 0..8 {
            a.session.add("you", format!("request {n}"));
            a.session.messages.extend([
                json!({"role":"user","content":format!("request {n}")}),
                json!({"role":"assistant","content":[{"type":"text","text":"x".repeat(12_000)}]}),
            ]);
        }
        a.persist().unwrap();
        a.command("/compact keep the parser API").unwrap();
        assert!(a.running.is_some());
        finish(&mut a);
        let checkpoint = a.session.checkpoint.as_ref().unwrap();
        assert_eq!(checkpoint.method, "demo");
        assert_eq!(checkpoint.note, "keep the parser API");
        assert!(a.store.load(&a.session.id).unwrap().checkpoint.is_some());
        assert!(a.session.messages.len() < 16);
        a.command("/compact auto off").unwrap();
        assert!(!a.session.auto_compact);
        assert!(screen(&mut a, 132, 42).contains("auto-compact off"));
    }
    #[test]
    fn multiline_composer_follows_the_cursor() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.input_set("FIRST\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\nLAST");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        // One Home reaches the start of the line; the draft shows six rows.
        assert!(text.contains("LAST") && !text.contains("FIRST"));
        a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("FIRST"));
        // End, End: the end of the line, then of the draft.
        a.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
            .unwrap();
        a.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("LAST"));
    }
}
