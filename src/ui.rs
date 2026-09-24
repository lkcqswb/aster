use crate::{
    agent::{self, Event, Running},
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
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
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

const BG: Color = Color::Rgb(17, 19, 24);
const FG: Color = Color::Rgb(222, 218, 208);
const DIM: Color = Color::Rgb(116, 124, 133);
const JADE: Color = Color::Rgb(164, 196, 168);
const GOLD: Color = Color::Rgb(217, 182, 131);
const LINE: Color = Color::Rgb(49, 54, 62);
const RED: Color = Color::Rgb(213, 144, 145);
const COMMANDS: &[(&str, &str)] = &[
    ("/new", "Start a fresh conversation"),
    ("/sessions", "Find and resume a session"),
    ("/rename", "Name this conversation"),
    ("/fork", "Branch the conversation"),
    ("/model", "Choose live MiniMax or demo"),
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
    ("/compact", "Checkpoint context with an optional note"),
    ("/checkpoint", "Inspect the latest context checkpoint"),
    ("/restore", "Restore archived context as a new conversation"),
    ("/export", "Save a readable transcript"),
    ("/tools", "Expand or collapse tool details"),
    ("/mood", "neutral, happy, heart, angry"),
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

struct Approval {
    tool: String,
    preview: String,
    scroll: u16,
    answer: crossbeam_channel::Sender<bool>,
}
enum Popup {
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
    },
    Delete,
    Approval(Approval),
}
pub struct App {
    cfg: Config,
    cli: Cli,
    store: Store,
    pub session: Session,
    input: String,
    cursor: usize,
    stream: String,
    running: Option<Running>,
    popup: Option<Popup>,
    // Keep one decision alive while read-only views replace each other.
    inspection_return: Option<Box<Popup>>,
    scroll: usize,
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
        Ok(Self {
            cfg,
            cli,
            store,
            session,
            input: String::new(),
            cursor: 0,
            stream: String::new(),
            running: None,
            popup: None,
            inspection_return: None,
            scroll: 0,
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
            quit: false,
            quit_started: None,
        })
    }
    fn notify(&mut self, text: impl Into<String>) {
        self.notice = text.into();
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
        self.input = text.into();
        self.cursor = self.input.len();
        self.selection = 0;
    }
    fn paste(&mut self, text: &str) {
        let text = clean(text);
        if let Some(Popup::Actions(menu)) = &mut self.popup {
            menu.paste(&text);
        } else if let Some(Popup::Project(nav)) = &mut self.popup {
            nav.paste(&text);
        } else if let Some(
            Popup::Resources { query, index, .. } | Popup::Sessions { query, index, .. },
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
        } else if self.popup.is_none() && self.input.len() + text.len() <= 32_000 {
            self.input.insert_str(self.cursor, &text);
            self.cursor += text.len();
            self.last_type = Instant::now();
        }
    }
    fn suggestions(&self) -> Vec<(&'static str, &'static str)> {
        let input = self.input.trim();
        if !input.starts_with('/') || input.contains(' ') {
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
        self.scroll = 0;
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
                | "/checkpoint"
                | "/skills"
                | "/files"
                | "/find"
                | "/prompts"
                | "/drop"
        ) && self.busy_guard()
        {
            return Ok(());
        }
        match cmd {
   "/context"=>{let catalog=crate::context::discover(&self.cfg.project);let rules=instructions::load(&self.cfg.project)?;self.info("Context beside 弄玉",format!("{} provider messages · {} KB of saved content\n{} visible transcript entries · {} waiting messages\n\nAGENTS.md sources\n{}\n\n{} skills available · {} prompts available\n\nSkills used this turn\n{}\n\nAttached files this turn\n{}\n\nUse @path or @{{path with spaces}} to attach a project file.\nUse @path:10-30 for selected lines.\nFull skill text is loaded only on invocation or read_skill.\n/compact [note] saves a recoverable checkpoint. /checkpoint shows it.\nByte counts describe content, not exact model tokens.",self.session.messages.len(),serde_json::to_vec(&self.session.messages)?.len()/1000,self.session.entries.len(),self.session.pending.len(),rules.iter().map(|r|r.path.display().to_string()).collect::<Vec<_>>().join("\n"),catalog.skills.len(),catalog.prompts.len(),self.session.work.skills.join("\n"),self.session.work.context_files.join("\n")));},
   "/skills"=>{if arg.is_empty(){self.show_resources(true)}else{let v=crate::context::discover(&self.cfg.project).read_skill(arg,"SKILL.md",true)?;self.info("Skills beside 弄玉",format!("{}\n\n{}\n\n/skill {} request · invoke",v["directory"].as_str().unwrap_or(""),v["content"].as_str().unwrap_or(""),arg));}},
   "/prompts"=>self.show_resources(false),
   "/skill"|"/prompt"=>{if arg.is_empty(){bail!("Add a resource name and your request")};let invocation=format!("{cmd} {arg}");crate::context::prepare(&self.cfg.project,&invocation,&crate::context::discover(&self.cfg.project))?;self.submit(invocation)?;},
   "/reload"=>{let rules=instructions::load(&self.cfg.project)?;let catalog=crate::context::discover(&self.cfg.project);self.info("Project resources refreshed",format!("{} AGENTS.md files · {} skills · {} prompts\n\nResources are also reloaded at the start of each turn.\n{}",rules.len(),catalog.skills.len(),catalog.prompts.len(),catalog.warnings.join("\n")));},
   "/steer"=>self.enqueue(arg.into(),Delivery::Steer)?,
   "/follow"=>self.enqueue(arg.into(),Delivery::FollowUp)?,
   "/queue"=>{let text=if self.session.pending.is_empty(){"No messages waiting.\n\nWhile working: Enter adds a direction; Alt+Enter queues the next task.\n/steer MESSAGE · /follow MESSAGE".into()}else{format!("{}\n\n/next runs the next message when idle.\n/drop ID removes a waiting message.\nStopping preserves the queue; it does not run automatically after an error or restart.",self.session.pending.iter().map(|m|format!("{} · {}\n{}\n",m.id,if m.delivery==Delivery::Steer{"direction"}else{"next task"},m.text)).collect::<Vec<_>>().join("\n"))};self.info("Messages waiting for 弄玉",text);},
   "/next"=>self.run_next()?,
   "/drop"=>{let Some(index)=self.session.pending.iter().position(|m|m.id==arg)else{bail!("Use /queue to find the message ID")};let item=&self.session.pending[index];if item.delivery==Delivery::Steer&&let Some(r)=&self.running {let mut q=r.steering.lock().unwrap();let Some(at)=q.iter().position(|m|m.id==arg)else{bail!("That direction has already reached the agent")};q.remove(at);}self.session.pending.remove(index);self.persist()?;self.notify("Waiting message removed");},
   "/together"=>self.show_actions(),
   "/work"=>self.show_work(),
   "/files"|"/find"=>self.inspect(Popup::Project(Box::new(crate::navigator::Navigator::new(self.cfg.project.clone(),if cmd=="/files"{crate::navigator::Mode::Files}else{crate::navigator::Mode::Search},arg.into())))),
   "/checks"=>self.info("Checks beside 弄玉",self.session.work.checks_summary()),
   "/run"=>{if arg.is_empty(){bail!("Use /run followed by a shell command")};self.submit(format!("/run {arg}"))?;},
   "/output"=>self.info("Command output · 弄玉",self.session.work.command.as_ref().map(|c|c.summary()).unwrap_or_else(||"No command has run in this turn.\nCommand output appears here while it runs.\nF4 opens this view.".into())),
   "/recover"=>{let command=self.session.work.command.as_ref().context("No failed command to recover")?;if command.running || (command.exit_code==Some(0)&&!command.stopped&&!command.timed_out){bail!("The latest command has not failed")};let prompt=format!("Help me recover from the latest command failure. Share a concise plan with update_plan. Inspect the actual output and relevant project files, explain the cause supported by evidence, then make a focused fix and run an appropriate check. Do not blindly repeat the same command. Respect my permission settings.\n\n{}",command.summary());self.submit(prompt)?;},
   "/review"=>{let text=if self.session.work.diffs.is_empty(){"No file edits recorded for this turn. Shell changes may require a git diff.\n\n/work shows the plan and actual checks.".into()}else{self.session.work.diffs.iter().map(|(_,d)|d.as_str()).collect::<Vec<_>>().join("\n\n")};self.info("Review changes · 弄玉",text);},
   "/new"|"/clear"=>self.new_session(arg)?,
   "/sessions"|"/resume"=>{if !arg.is_empty(){let s=self.store.load(arg)?;if s.project!=self.cfg.project{bail!("Session belongs to a different project")};self.session=s;self.scroll=0;self.persist()?;}else{self.popup=Some(Popup::Sessions{items:self.store.list(&self.cfg.project)?,query:String::new(),index:0});}},
   "/rename"=>{if arg.is_empty(){bail!("Use /rename followed by a title")};self.session.title=arg.chars().take(120).collect();self.session.updated=chrono::Utc::now().to_rfc3339();self.persist()?;},
   "/fork"=>{self.session=self.session.fork();if !arg.is_empty(){self.session.title=arg.into()};self.session.add("notice","Forked the conversation. This session shares the project files; no files were rolled back.");self.persist()?;self.notify("New branch saved. Project files are shared.");},
   "/model"|"/models"=>{match arg{""=>self.info("Choose an agent",format!("Current: {}\n\n/model live     MiniMax · real model\n/model demo     Offline scripted demo\n/model NAME     Use a specific MiniMax model\n\nModel changes take effect on the next message.",if self.session.demo{"offline demo"}else{&self.session.model})),"demo"=>{self.session.demo=true;self.notify("Offline demo · no API calls");},"live"=>{self.session.demo=false;self.notify(format!("MiniMax · {}",self.session.model));},name=>{if name.len()>120{bail!("Model name is too long")};self.session.model=name.into();self.session.demo=false;self.notify(format!("Model: {name}"));}}self.persist()?;},
   "/agents"=>{let rules=instructions::load(&self.cfg.project)?;self.info("Project instructions",if rules.is_empty(){"No AGENTS.md found. /init creates project guidance.".into()}else{instructions::format(&rules)});},
   "/init"=>{let p=self.cfg.project.join("AGENTS.md");if p.exists(){self.info("AGENTS.md",fs::read_to_string(p)?)}else{tools::path(&self.cfg.project,"AGENTS.md")?;crate::session::private_write(&p,b"# Project guidance\n\n- Inspect relevant files before making changes.\n- Keep changes focused on the requested task.\n- Run the project's relevant checks and report actual results.\n- Do not read or publish credentials.\n\n## Build and test\n\nAdd this project's build and test commands here.\n")?;self.notify("Created AGENTS.md. Use /agents to inspect it.");}},
   "/plan"=>{self.session.mode="plan".into();self.persist()?;self.notify("Plan mode · read and discuss, no writes or shell commands");},
   "/build"=>{self.session.mode="build".into();self.persist()?;self.notify("Build mode · tools follow your permission setting");},
   "/permissions"=>{if !matches!(arg,"ask"|"allow"|"deny"){bail!("Use /permissions ask, allow, or deny")};self.cli.permissions=arg.into();self.notify(format!("Permissions: {arg} · applies to file writes and shell commands"));},
   "/check"=>self.local_check(arg)?,

   "/compact"=>self.compact(arg)?,
   "/checkpoint"=>self.info("Context checkpoint · 弄玉",self.session.checkpoint.as_ref().map(|c|c.report()).unwrap_or_else(||"No checkpoint yet. /compact [note] archives full context and keeps bounded recent exchanges with local historical excerpts. It makes no model request.".into())),
   "/restore"=>{if arg.is_empty(){bail!("Use /restore followed by the ID shown in /checkpoint")};self.persist()?;self.session=self.store.restore_checkpoint(arg,&self.cfg.project)?;self.scroll=0;self.stream.clear();self.notify("Full context restored as a new conversation. Project files are shared.");},
   "/export"=>{let p=self.store.export(&self.session)?;self.notify(format!("Saved {}",p.display()));},
   "/tools"=>{self.show_tools = !self.show_tools;self.notify(if self.show_tools{"Tool details expanded"}else{"Tool details collapsed"});},
   "/mood"=>{if !matches!(arg,"neutral"|"happy"|"heart"|"angry"){bail!("Use /mood neutral, happy, heart, or angry")};self.mood=arg.into();},
   "/look"=>{if let Some(c)=&self.companion{c.motion(&self.state,&self.mood,false,true)}self.reaction=Some(("listening".into(),Instant::now()));},
   "/pet"=>{match arg { "off" => {self.companion=None;self.portrait=Shared::default();}, "on"|"retry"|"restart" => {self.companion=None;self.portrait=Shared::default();self.companion=Some(Companion::start(self.cfg.clone()));}, "" if self.companion.is_some() => {self.companion=None;self.portrait=Shared::default();}, "" => {self.companion=Some(Companion::start(self.cfg.clone()));}, _ => self.notify("Use /pet on, /pet off, or /pet retry") }self.last_image=None;},
   "/demo"=>{self.session.demo=true;self.submit(if arg=="work"{"companion demo".into()}else if arg.starts_with("evidence"){format!("evidence demo {}",arg.strip_prefix("evidence").unwrap_or(""))}else if arg.starts_with("command"){format!("command demo {}",arg.strip_prefix("command").unwrap_or(""))}else{"demo task".into()})?;},
   "/status"=>self.info("Aster · session status",format!("Session    {}\nProject    {}\nModel      {}\nProvider   {}\nMode       {} · permissions {}\nUsage      {} input / {} output tokens\nTools      {}\nChecks     {} passed / {} total\n\nGraphics   {}\nLive2D     {}\nFrames     {}\n\n{}\n\nTurn limits: 12 requests · 24 tools · 180 active seconds\n2,048 output tokens/request · 12,000 output tokens/turn\nDecision waits pause the timer (up to 15 minutes each).\nNo automatic retries. Token limits are not a currency budget.",self.session.id,self.cfg.project.display(),self.session.model,if self.session.demo{"scripted demo"}else{"MiniMax"},self.session.mode,self.cli.permissions,self.session.input_tokens,self.session.output_tokens,self.session.tools,self.session.checks.iter().filter(|c|c.passed).count(),self.session.checks.len(),self.graphics.name(),self.portrait.status,self.portrait.frames,serde_json::to_string_pretty(&self.portrait.info)?)),
   "/stop"=>self.stop(),
   "/delete"=>self.popup=Some(Popup::Delete),
   "/help"=>self.info("Make yourself at home",format!("{}\n\nEnter sends / steers · Alt+Enter queues · Ctrl+G redirects\nCtrl+J inserts a line · Esc stops\nCtrl+P opens sessions · Ctrl+K opens commands\nPage Up/Down scroll · Ctrl+T shows tools\nCtrl+C saves and quits · F1 or click 弄玉 for local task controls\n\n/new [title] · /rename TITLE · /fork [title]\n/resume ID · /check FILE [expected JSON]\n\nThe model is an AI companion. Speaking motion follows text activity; no voice is synthesized.",COMMANDS.iter().map(|(a,b)|format!("{a:15} {b}")).collect::<Vec<_>>().join("\n"))),
   "/quit"|"/exit"=>self.request_quit(),
   _=>bail!("Unknown command. Type / to see available commands."),
  }
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
        self.notice = self.session.work.verdict().into();
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
        self.scroll = 0;
        self.state = "thinking".into();
        self.notice.clear();
        Ok(())
    }
    fn stop(&mut self) {
        if let Some(r) = &self.running {
            r.cancel.store(true, Ordering::Relaxed);
            self.notify("Stopping… an already submitted request may still consume tokens.");
        } else {
            self.input_set("");
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
                        self.notice =
                            "A check or command failed · F4 output · /work details".into();
                        self.reaction = Some(("concerned".into(), Instant::now()));
                    } else if self.session.work.has_stale_checks() {
                        self.notice =
                            "Edits changed the project after checks · F5 to review".into();
                        self.reaction = Some(("concerned".into(), Instant::now()));
                    }
                    self.running = None;
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
        } else if matches!(&self.popup,Some(Popup::Project(nav)) if nav.reading() || nav.busy()) {
            "reading"
        } else if self.last_type.elapsed() < Duration::from_secs(2) && !self.input.is_empty() {
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
            self.request_quit();
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
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            menu.query.clear();
                            menu.index = 0;
                        }
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
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
                            if self.input.len() + reference.len() + 1 > 32_000 {
                                self.notify(
                                    "The draft is full. Shorten it before attaching a file.",
                                );
                                self.popup = Some(Popup::Project(nav));
                            } else {
                                let text = if self.input.trim().is_empty() {
                                    format!("{reference} ")
                                } else {
                                    format!("{} {reference} ", self.input.trim_end())
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
                        KeyCode::Enter => {
                            if let Some(resource) = filtered.get(index) {
                                let prepared = format!(
                                    "/{} {} {}",
                                    if skills { "skill" } else { "prompt" },
                                    resource.name,
                                    self.input
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
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
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
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
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
                        KeyCode::Char(c) if input.len() < 2000 => input.push(c),
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
                        a.scroll = a.scroll.saturating_add(5);
                        self.popup = Some(Popup::Approval(a));
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        a.scroll = a.scroll.saturating_sub(5);
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
                        KeyCode::Down | KeyCode::PageDown => scroll = scroll.saturating_add(4),
                        KeyCode::Up | KeyCode::PageUp => scroll = scroll.saturating_sub(4),
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
                } => {
                    let filtered = items
                        .iter()
                        .filter(|s| {
                            s.title.to_lowercase().contains(&query.to_lowercase())
                                || s.id.contains(&query)
                        })
                        .collect::<Vec<_>>();
                    match key.code {
                        KeyCode::Esc => return Ok(()),
                        KeyCode::Down => index = (index + 1).min(filtered.len().saturating_sub(1)),
                        KeyCode::Up => index = index.saturating_sub(1),
                        KeyCode::Enter => {
                            if let Some(s) = filtered.get(index) {
                                self.session = (*s).clone();
                                self.scroll = 0;
                                self.stream.clear();
                                self.persist()?;
                            }
                            return Ok(());
                        }
                        KeyCode::Backspace => {
                            query.pop();
                            index = 0
                        }
                        KeyCode::Char(c) => {
                            query.push(c);
                            index = 0
                        }
                        _ => {}
                    }
                    self.popup = Some(Popup::Sessions {
                        items,
                        query,
                        index,
                    });
                }
            }
            return Ok(());
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') => {
                    if !self.busy_guard() {
                        self.popup = Some(Popup::Sessions {
                            items: self.store.list(&self.cfg.project)?,
                            query: String::new(),
                            index: 0,
                        });
                    }
                }
                KeyCode::Char('k') => self.input_set("/"),
                KeyCode::Char('t') => self.show_tools = !self.show_tools,
                KeyCode::Char('j') => {
                    self.input.insert(self.cursor, '\n');
                    self.cursor += 1;
                }
                KeyCode::Char('a') => self.cursor = 0,
                KeyCode::Char('e') => self.cursor = self.input.len(),
                KeyCode::Char('u') => self.input_set(""),
                _ => {}
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                let input = self.input.trim().to_string();
                self.enqueue(input, Delivery::FollowUp)?;
                self.input_set("");
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor, '\n');
                self.cursor += 1;
            }
            KeyCode::Enter => {
                let suggestions = self.suggestions();
                if !suggestions.is_empty()
                    && self.input != suggestions[self.selection.min(suggestions.len() - 1)].0
                {
                    let cmd = suggestions[self.selection.min(suggestions.len() - 1)].0;
                    self.input_set(cmd);
                    return Ok(());
                }
                let input = self.input.trim().to_string();
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
                let options = self.suggestions();
                if let Some((cmd, _)) =
                    options.get(self.selection.min(options.len().saturating_sub(1)))
                {
                    self.input_set(format!("{cmd} "));
                }
            }
            KeyCode::Esc => self.stop(),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let at = self.input[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input.replace_range(at..self.cursor, "");
                    self.cursor = at;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    let len = self.input[self.cursor..].chars().next().unwrap().len_utf8();
                    self.input.replace_range(self.cursor..self.cursor + len, "");
                }
            }
            KeyCode::Left => {
                self.cursor = self.input[..self.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                if let Some(c) = self.input[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8()
                }
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.len(),
            KeyCode::Up => {
                if !self.suggestions().is_empty() {
                    self.selection = self.selection.saturating_sub(1);
                } else {
                    self.scroll += 3;
                }
            }
            KeyCode::Down => {
                let n = self.suggestions().len();
                if n > 0 {
                    self.selection = (self.selection + 1).min(n - 1);
                } else {
                    self.scroll = self.scroll.saturating_sub(3);
                }
            }
            KeyCode::PageUp => self.scroll += 12,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(12),
            KeyCode::Char(c) if !c.is_control() && self.input.len() < 32_000 => {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                self.last_type = Instant::now();
                self.selection = 0;
            }
            _ => {}
        }
        Ok(())
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
            horizontal: 2,
            vertical: 1,
        });
        let project = self
            .cfg
            .project
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let header = Line::from(vec![
            Span::styled("✦ aster", style(JADE).add_modifier(Modifier::BOLD)),
            Span::styled(format!("   /   {}", clean(&project)), style(DIM)),
            Span::styled(
                format!(
                    "   {}",
                    if self.session.mode == "plan" {
                        "PLAN"
                    } else {
                        ""
                    }
                ),
                style(GOLD),
            ),
        ]);
        f.render_widget(
            Paragraph::new(header),
            Rect::new(area.x, area.y, area.width, 1),
        );
        let model = if self.session.demo {
            "offline demo"
        } else {
            &self.session.model
        };
        let label = format!("{}  ·  {}", model, &self.session.id[..6]);
        let len = label.width() as u16;
        if area.width > 88 {
            f.render_widget(
                Paragraph::new(label).style(style(DIM)),
                Rect::new(area.right().saturating_sub(len), area.y, len, 1),
            );
        }
        let input_lines = wrap(&self.input, area.width.saturating_sub(4) as usize);
        let composer_h = (input_lines.len() as u16).clamp(1, 5) + 3;
        let body = Rect::new(
            area.x,
            area.y + 3,
            area.width,
            area.height.saturating_sub(composer_h + 5),
        );
        let pet_width = if self.companion.is_some() || self.portrait.frame.is_some() {
            if area.width >= 96 {
                area.width * 34 / 100
            } else if area.width >= 72 {
                23
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
                .saturating_sub(pet_width + if pet_width > 0 { 4 } else { 0 }),
            body.height,
        );
        if self.session.entries.is_empty() && self.stream.is_empty() {
            self.welcome(f, chat)
        } else {
            self.conversation(f, chat)
        }
        if pet_width > 0 {
            let pet = Rect::new(body.right() - pet_width, body.y, pet_width, body.height);
            self.draw_pet(f, pet);
        }
        let composer_y = area.bottom().saturating_sub(composer_h + 1);
        let title = tools::clip(&clean(&self.session.title), 60);
        let footer_title = format!(
            "{}  ·  {}",
            title,
            if self.running.is_some() {
                &self.state
            } else {
                "ready"
            }
        );
        f.render_widget(
            Paragraph::new(footer_title).style(style(DIM)),
            Rect::new(area.x, composer_y.saturating_sub(1), area.width, 1),
        );
        f.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(style(LINE)),
            Rect::new(area.x, composer_y, area.width, composer_h),
        );
        let input_area = Rect::new(
            area.x + 3,
            composer_y + 1,
            area.width.saturating_sub(4),
            composer_h - 2,
        );
        f.render_widget(
            Paragraph::new("›").style(style(JADE)),
            Rect::new(area.x, composer_y + 1, 2, 1),
        );
        let prefix = wrap(&self.input[..self.cursor], input_area.width as usize);
        let input_start = prefix.len().saturating_sub(input_area.height as usize);
        if self.input.is_empty() {
            f.render_widget(
                Paragraph::new(if self.running.is_some() {
                    "Add a direction · Enter steer · Alt+Enter next task"
                } else {
                    "和弄玉说说，你想做什么？"
                })
                .style(style(DIM)),
                input_area,
            );
        } else {
            f.render_widget(
                Paragraph::new(
                    input_lines[input_start..]
                        .iter()
                        .take(input_area.height as usize)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
                .style(style(FG)),
                input_area,
            );
        }
        let bottom = if self.notice.is_empty() && !self.session.pending.is_empty() {
            format!(
                "{} messages waiting · /queue inspect · Esc stops and keeps the queue",
                self.session.pending.len()
            )
        } else if self.notice.is_empty() {
            "↵ send   / commands   F1 together   F6 files   Ctrl+P sessions   Esc stop".into()
        } else {
            clean(&self.notice)
        };
        f.render_widget(
            Paragraph::new(bottom).style(style(DIM)),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
        if self.popup.is_none() {
            let y = (prefix.len() - 1 - input_start) as u16;
            let x = prefix
                .last()
                .map(|s| s.width())
                .unwrap_or(0)
                .min(input_area.width.saturating_sub(1) as usize) as u16;
            f.set_cursor_position((input_area.x + x, input_area.y + y));
        }
        let options = self.suggestions();
        if !options.is_empty() && self.popup.is_none() {
            let count = options.len().min(8) as u16;
            let w = area.width.min(74);
            let r = Rect::new(area.x, composer_y.saturating_sub(count + 2), w, count + 2);
            f.render_widget(Clear, r);
            f.render_widget(
                Block::default()
                    .style(style(FG))
                    .borders(Borders::LEFT)
                    .border_style(style(JADE)),
                r,
            );
            if !r.intersection(self.image_area).is_empty() {
                self.image_area = Rect::default();
            }
            let start = self.selection.saturating_sub(7);
            for (i, (name, help)) in options.iter().skip(start).take(8).enumerate() {
                let chosen = i + start == self.selection;
                let text = format!(" {} {:14} {}", if chosen { "›" } else { " " }, name, help);
                f.render_widget(
                    Paragraph::new(text).style(style(if chosen { JADE } else { DIM })),
                    Rect::new(r.x + 1, r.y + i as u16 + 1, r.width - 2, 1),
                );
            }
        }
        self.action_rows.clear();
        if let Some(popup) = &self.popup {
            let decision = matches!(
                popup,
                Popup::Approval(_)
                    | Popup::Actions(_)
                    | Popup::Project(_)
                    | Popup::Question { .. }
                    | Popup::Redirect { .. }
                    | Popup::Resources { .. }
            ) || matches!(popup, Popup::Info{title,..} if title.starts_with("Working together") || title.starts_with("Review changes") || title.starts_with("Messages waiting") || title.starts_with("Context beside") || title.starts_with("Skills beside") || title.starts_with("Command output") || title.starts_with("Checks beside") || title.starts_with("Context checkpoint"));
            let side_by_side = decision && pet_width > 0 && chat.width >= 42;
            if side_by_side {
                f.render_widget(Clear, chat);
                f.render_widget(Block::default().style(style(FG)), chat);
            }
            if !side_by_side {
                self.image_area = Rect::default();
            }
            self.action_rows = Self::draw_popup(
                f,
                popup,
                self.inspection_return.is_some(),
                if side_by_side {
                    Rect::new(
                        chat.x,
                        area.y + 2,
                        chat.width,
                        area.height.saturating_sub(7),
                    )
                } else {
                    area
                },
            );
        }
    }
    fn welcome(&self, f: &mut Frame, r: Rect) {
        let y = r.y + r.height.saturating_sub(15) / 2;
        let w = r.width;
        let texts = [
            ("NONGYU / 弄玉", JADE),
            ("", FG),
            ("A little company.", FG),
            ("A place to make things.", FG),
            ("", FG),
            ("Aster，今天想一起做什么？", FG),
            ("", FG),
            ("Tell me what you have in mind.", DIM),
            ("I’ll stay with the work, from idea to check.", DIM),
            ("", FG),
            ("/sessions   pick up where you left off", DIM),
            ("/agents     the rules of this project", DIM),
            ("/demo       try a real file + check", DIM),
        ];
        for (i, (t, c)) in texts.into_iter().enumerate() {
            if y + (i as u16) < r.bottom() {
                f.render_widget(
                    Paragraph::new(t).style(style(c)),
                    Rect::new(r.x + 1, y + i as u16, w.saturating_sub(2), 1),
                );
            }
        }
    }
    fn conversation(&self, f: &mut Frame, r: Rect) {
        let mut lines = vec![];
        let width = r.width.saturating_sub(2) as usize;
        let mut entries = self.session.entries.clone();
        if !self.stream.is_empty() {
            entries.push(Entry {
                role: "nongyu".into(),
                text: self.stream.clone(),
            });
        }
        for e in entries {
            if e.role == "tool"
                && !self.show_tools
                && (e.text.starts_with("update_plan ") || e.text.starts_with("ask_user "))
            {
                continue;
            }
            let (label, color) = match e.role.as_str() {
                "you" => ("YOU", DIM),
                "nongyu" => ("弄玉", JADE),
                "tool" => ("", DIM),
                _ => ("•", GOLD),
            };
            if e.role == "tool" {
                let first = e.text.lines().next().unwrap_or("");
                let failed = first.contains("failed");
                lines.push(line(
                    format!("  {} {}", if failed { "!" } else { "·" }, clean(first)),
                    if failed { RED } else { DIM },
                ));
                if self.show_tools {
                    for text in wrap(&e.text, width.saturating_sub(3)).into_iter().skip(1) {
                        lines.push(line(format!("    {text}"), DIM));
                    }
                }
                lines.push(line("", FG));
                continue;
            }
            lines.push(line(label, color));
            for text in wrap_prose(&e.text, width) {
                lines.push(line(
                    format!("  {text}"),
                    if e.role == "notice" { GOLD } else { FG },
                ));
            }
            lines.push(line("", FG));
        }
        if self.running.is_some() && self.stream.is_empty() {
            let ticks = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
            let i =
                (chrono::Utc::now().timestamp_millis() / 120).unsigned_abs() as usize % ticks.len();
            lines.push(line(format!("{} 弄玉 · {}", ticks[i], self.state), JADE));
        }
        let max = lines.len().saturating_sub(r.height as usize);
        let start = max.saturating_sub(self.scroll);
        f.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(start)
                    .take(r.height as usize)
                    .collect::<Vec<_>>(),
            ),
            r,
        );
    }
    fn draw_pet(&mut self, f: &mut Frame, r: Rect) {
        let compact = r.height < 20;
        f.render_widget(
            Paragraph::new("弄玉").style(style(FG).add_modifier(Modifier::BOLD)),
            Rect::new(r.x + 1, r.y, r.width.saturating_sub(2), 1),
        );
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
        } else if self.last_type.elapsed() < Duration::from_secs(2) && !self.input.is_empty() {
            "listening"
        } else if self.session.work.has_failures() {
            "a check needs attention"
        } else if self.session.work.has_stale_checks() {
            "edits need a fresh check"
        } else {
            "here with you"
        };
        f.render_widget(
            Paragraph::new(format!("◌ {state}")).style(style(JADE)),
            Rect::new(
                r.x + 1,
                r.y + if compact { 1 } else { 2 },
                r.width.saturating_sub(2),
                1,
            ),
        );
        let card_height = if r.height >= 24 {
            8
        } else if r.height >= 16 {
            4
        } else {
            1
        };
        let height = r
            .height
            .saturating_sub(if compact { 3 } else { 5 } + card_height);
        let width = r.width.min((f64::from(height) * 1.36) as u16);
        let height = height.min((f64::from(width) / 1.36) as u16);
        let area = Rect::new(
            r.x + (r.width - width) / 2,
            r.y + if compact { 2 } else { 4 },
            width,
            height,
        );
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
                    "Live2D is hidden".into()
                })
                .wrap(Wrap { trim: true })
                .style(style(DIM)),
                area,
            );
        }
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
                JADE,
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
                lines.push(line(
                    format!(
                        "{}/{} steps · {} {}",
                        work.steps
                            .iter()
                            .filter(|s| s.status == crate::work::StepStatus::Done)
                            .count(),
                        work.steps.len(),
                        work.changed.len(),
                        if work.changed.len() == 1 {
                            "file"
                        } else {
                            "files"
                        }
                    ),
                    DIM,
                ));
            }
        }
        if card_height >= 4 {
            lines.push(line(
                work.verdict(),
                if work.has_failures() || work.has_stale_checks() {
                    GOLD
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
                    format!("{:.1}s · {}", command.elapsed_ms as f64 / 1000., last),
                    DIM,
                ));
            } else {
                lines.push(line("", DIM));
            }
        }
        lines.push(line(
            if work.command.is_some() {
                "F1 together · F4 output"
            } else {
                "F1 together · F3 review"
            },
            JADE,
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
    fn draw_popup(
        f: &mut Frame,
        p: &Popup,
        returning: bool,
        area: Rect,
    ) -> Vec<(Rect, crate::actions::Action)> {
        let width = area.width.saturating_sub(4).min(88);
        let desired_height = if let Popup::Resources { items, .. } = p {
            11 + 3 * items.len().min(5) as u16
        } else {
            28
        };
        let height = area.height.saturating_sub(4).min(desired_height);
        let r = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        f.render_widget(Clear, r);
        f.render_widget(
            Block::default()
                .style(style(FG))
                .borders(Borders::ALL)
                .border_style(style(LINE)),
            r,
        );
        let inner = r.inner(Margin {
            horizontal: 3,
            vertical: 2,
        });
        if let Popup::Actions(menu) = p {
            let filtered = menu.filtered();
            let visible = (inner.height.saturating_sub(7) / 2).max(1) as usize;
            let mut rows = vec![];
            let mut write = |text: String, color, y| {
                if y < inner.bottom() {
                    f.render_widget(
                        Paragraph::new(text).style(style(color)),
                        Rect::new(inner.x, y, inner.width, 1),
                    );
                }
            };
            write("Together with 弄玉".into(), JADE, inner.y);
            write("Local controls · no model request".into(), DIM, inner.y + 2);
            write(format!("Find: {}", menu.query), FG, inner.y + 3);
            for (row, (index, choice)) in filtered
                .iter()
                .enumerate()
                .skip(menu.index.saturating_sub(visible - 1))
                .take(visible)
                .enumerate()
            {
                let y = inner.y + 5 + row as u16 * 2;
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
                    inner.y + 5,
                );
            }
            write(
                format!(
                    "↑↓ or click · Enter open · Esc back · {}/{}",
                    if filtered.is_empty() {
                        0
                    } else {
                        menu.index + 1
                    },
                    filtered.len()
                ),
                DIM,
                inner.bottom().saturating_sub(1),
            );
            if returning {
                f.render_widget(
                    Paragraph::new("Decision still waiting · Esc back · Ctrl+G redirect")
                        .style(style(GOLD)),
                    Rect::new(inner.x, inner.bottom(), inner.width, 1),
                );
            }
            return rows;
        }
        let (title,text,scroll)=match p{
   Popup::Actions(_)=>unreachable!("Action menu is rendered above"),
   Popup::Project(nav)=>nav.view(inner.width as usize,inner.height as usize),
   Popup::Resources{items,skills,query,index}=>{let filtered=items.iter().filter(|r|format!("{} {}",r.name,r.description).to_lowercase().contains(&query.to_lowercase())).collect::<Vec<_>>();let visible=(inner.height.saturating_sub(6)/3).max(1) as usize;let mut text=format!("Find: {query}\n\n");for (i,r) in filtered.iter().enumerate().skip(index.saturating_sub(visible-1)).take(visible){text+=&format!("{} {}{}\n  {}\n\n",if i==*index{"›"}else{" "},r.name,if r.manual_only{" · explicit only"}else{""},{let lines=wrap_prose(&r.description.replace('\n'," "),inner.width.saturating_sub(4) as usize);format!("{}{}",lines.first().cloned().unwrap_or_default(),if lines.len()>1{"…"}else{""})});}if filtered.is_empty(){text+="No matching resources.\n";}text+="\n↑↓ choose · Enter prepare · F1 inspect · Esc close";(if *skills{"Skills beside 弄玉"}else{"Reusable prompts"}.into(),text,0)},
   Popup::Redirect{input,..}=>("弄玉 · change direction".into(),format!("Tell me what to change.\nPending actions will be cancelled when you send.\n\n› {input}\n\nEnter send · Esc return to the decision"),0),
   Popup::Question{question,options,input,..}=>("弄玉 · a question for you".into(),format!("{}\n\n{}\n\nOr type an answer:\n› {}",question,options.iter().enumerate().map(|(i,o)|format!("[{}] {o}",i+1)).collect::<Vec<_>>().join("\n"),input),0),
   Popup::Info{title,text,scroll}=>(title.clone(),text.clone(),*scroll),
   Popup::Approval(a)=>(format!("Allow {}?",a.tool),a.preview.clone(),a.scroll),
   Popup::Delete=>("Delete this conversation?".into(),"The saved conversation will be removed.\nProject files stay in place.\n\n[y] delete    [n] keep    Esc cancels".into(),0),
   Popup::Sessions{items,query,index}=>{let filtered=items.iter().filter(|s|s.title.to_lowercase().contains(&query.to_lowercase())||s.id.contains(query)).collect::<Vec<_>>();let mut text=format!("Find: {query}\n\n");for (i,s) in filtered.iter().enumerate().skip(index.saturating_sub((inner.height.saturating_sub(6)/2).max(1) as usize-1)).take((inner.height.saturating_sub(6)/2).max(1) as usize){text+=&format!("{} {}\n  {}  ·  {}\n",if i==*index{"›"}else{" "},s.title,&s.id[..6],if s.demo{"demo"}else{&s.model});}if filtered.is_empty(){text+="No matching sessions.\n"}text+="\n↑↓ choose    Enter resume    Esc close";("Your conversations".into(),text,0)}
  };
        f.render_widget(
            Paragraph::new(title).style(style(JADE).add_modifier(Modifier::BOLD)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        f.render_widget(
            Paragraph::new(clean(&text))
                .style(style(FG))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            Rect::new(
                inner.x,
                inner.y + 2,
                inner.width,
                inner.height.saturating_sub(3),
            ),
        );
        if returning {
            f.render_widget(
                Paragraph::new("Decision still waiting · Esc back · Ctrl+G redirect")
                    .style(style(GOLD)),
                Rect::new(inner.x, inner.bottom(), inner.width, 1),
            );
        }
        if matches!(p, Popup::Approval(_)) {
            f.render_widget(
                Paragraph::new("y allow · n deny · F2 plan · F6 files · Ctrl+G redirect")
                    .style(style(GOLD)),
                Rect::new(inner.x, inner.bottom(), inner.width, 1),
            );
        }
        if matches!(p, Popup::Question { .. }) {
            f.render_widget(
                Paragraph::new("1–5 choose · Enter send · F6 files · Esc dismiss")
                    .style(style(GOLD)),
                Rect::new(inner.x, inner.bottom(), inner.width, 1),
            );
        }
        vec![]
    }
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
                        write!(io::stdout(), "{}", app.graphics.clear())?;
                        terminal.resize(terminal.size()?.into())?;
                        app.last_image = None;
                    }
                    TermEvent::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            let up = matches!(mouse.kind, MouseEventKind::ScrollUp);
                            if app.popup.is_some() {
                                if let Err(e) = app.key(KeyEvent::new(
                                    if up { KeyCode::Up } else { KeyCode::Down },
                                    KeyModifiers::NONE,
                                )) {
                                    app.notify(e.to_string());
                                }
                            } else if up {
                                app.scroll += 3;
                            } else {
                                app.scroll = app.scroll.saturating_sub(3);
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
                if cli.prompt.as_ref().is_some_and(|p| p.starts_with("/run "))
                    && !s
                        .work
                        .command
                        .as_ref()
                        .is_some_and(|c| c.exit_code == Some(0) && !c.stopped && !c.timed_out)
                {
                    bail!("Local command did not succeed; inspect its saved output");
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
                "<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>",
                x as usize * cw,
                y as usize * ch + 14,
                color,
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
            texture_size: 2048,
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
        assert_eq!(a.input, "/skill review Explain this change 中文");
        assert!(a.session.messages.is_empty());
        a.command("/sessions").unwrap();
        a.paste("fresh");
        assert!(matches!(&a.popup, Some(Popup::Sessions{query,..}) if query=="fresh"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(a.input, "/skill review Explain this change 中文");
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
        assert_eq!(a.input, "Keep this composer draft");
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
        assert_eq!(a.input, "Keep my draft");
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
        assert_eq!(a.input, "Explain this @{notes.txt:1-80} ");
        assert!(a.popup.is_none());
        assert!(a.running.is_none());
        assert!(a.session.messages.is_empty());
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
        assert_eq!(a.input, "Keep my draft");
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
        assert_eq!(a.input, "Keep this draft");
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
        assert_eq!(a.input, "好Aster");
        a.command("/rename Test parent").unwrap();
        let parent = a.session.id.clone();
        a.command("/fork Test child").unwrap();
        assert_eq!(a.session.parent, Some(parent.clone()));
        assert_eq!(a.store.load(&parent).unwrap().title, "Test parent");
        assert_eq!(a.session.title, "Test child");
    }
    #[test]
    fn multiline_composer_follows_the_cursor() {
        let d = tempfile::tempdir().unwrap();
        let mut a = app(d.path());
        a.input_set("FIRST\nsecond\nthird\nfourth\nfifth\nsixth\nLAST");
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
        assert!(text.contains("FIRST"));
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
