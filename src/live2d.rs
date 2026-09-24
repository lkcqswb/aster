use crate::config::Config;
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

/// Portrait frames per second while Chromium keeps up.
const TARGET_FPS: f64 = 15.0;
/// Frames are spaced at least this multiple of their measured cost, so a slow renderer gets idle time.
const PACING_HEADROOM: f64 = 1.5;
/// The render thread always yields at least this long between frames.
const MIN_REST: Duration = Duration::from_millis(8);
/// The diagnostics file is written again at this frame so it records a measured frame rate.
const FPS_REPORT_FRAME: u64 = 45;
/// Bounds for each side of the rendered portrait, in pixels.
pub const VIEW_MIN: u32 = 160;
pub const VIEW_MAX: u32 = 1400;
/// The tuned portrait size, used until the UI reports its real pixel size.
const DEFAULT_VIEW: (u32, u32) = (420, 620);
const KITTY_IMAGE_ID: u32 = 731;

#[derive(Clone, Debug, Default)]
pub struct Motion {
    pub state: String,
    pub mood: String,
    pub tap: bool,
    pub look: bool,
    pub commands: Vec<Value>,
}

/// How a portrait frame is encoded. Kitty's `f=100` transmission accepts PNG only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameFormat {
    Png,
    Jpeg,
}
impl FrameFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
    fn page_name(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

/// Quadrant cells for one area size (columns, rows).
type CellCache = Option<((u16, u16), Arc<Vec<Quadrant>>)>;
#[derive(Default)]
struct FrameCache {
    rgb: OnceLock<Option<Arc<image::RgbImage>>>,
    cells: Mutex<CellCache>,
}

/// One encoded portrait image. Clones share its lazily decoded pixels and cell rendering.
#[derive(Clone)]
pub struct PortraitFrame {
    pub data: Vec<u8>,
    pub format: FrameFormat,
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    cache: Arc<FrameCache>,
}
impl PortraitFrame {
    pub fn new(data: Vec<u8>, format: FrameFormat, sequence: u64, width: u32, height: u32) -> Self {
        Self {
            data,
            format,
            sequence,
            width,
            height,
            cache: Arc::default(),
        }
    }
    /// Decoded pixels. Decoding happens once per frame, and only when something asks for it.
    pub fn rgb(&self) -> Option<Arc<image::RgbImage>> {
        self.cache
            .rgb
            .get_or_init(|| {
                let format = match self.format {
                    FrameFormat::Png => image::ImageFormat::Png,
                    FrameFormat::Jpeg => image::ImageFormat::Jpeg,
                };
                image::load_from_memory_with_format(&self.data, format)
                    .ok()
                    .map(|image| Arc::new(image.into_rgb8()))
            })
            .clone()
    }
    fn quadrants(&self, cols: u16, rows: u16) -> Option<Arc<Vec<Quadrant>>> {
        let mut cached = self.cache.cells.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((size, cells)) = cached.as_ref()
            && *size == (cols, rows)
        {
            return Some(cells.clone());
        }
        let cells = Arc::new(quadrant_cells(self.rgb()?.as_ref(), cols, rows));
        *cached = Some(((cols, rows), cells.clone()));
        Some(cells)
    }
}

#[derive(Clone, Default)]
pub struct Shared {
    pub frame: Option<PortraitFrame>,
    pub status: String,
    pub frames: u64,
    pub motion: Motion,
    pub info: Value,
    /// Measured portrait frames per second (0 until two frames have arrived).
    pub fps: f64,
}

/// What the UI tells the render thread besides the motion state.
struct Pane {
    view: (u32, u32),
}
impl Pane {
    fn new() -> Self {
        Self { view: DEFAULT_VIEW }
    }
}

/// Scale a pixel size into `VIEW_MIN..=VIEW_MAX` per side, keeping its aspect ratio where possible.
fn clamp_view(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let (w, h) = (f64::from(width), f64::from(height));
    let down = (f64::from(VIEW_MAX) / w.max(h)).min(1.0);
    let up = (f64::from(VIEW_MIN) / (w.min(h) * down)).max(1.0);
    let side = |v: f64| {
        (v * down * up)
            .round()
            .clamp(f64::from(VIEW_MIN), f64::from(VIEW_MAX)) as u32
    };
    Some((side(w), side(h)))
}

/// Frame pacing: aim for `TARGET_FPS`, but back off when frames cost more than the budget allows.
#[derive(Default)]
struct Pacing {
    cost_ms: Option<f64>,
    interval_ms: Option<f64>,
}
impl Pacing {
    fn ewma(previous: Option<f64>, sample: Duration) -> Option<f64> {
        let sample = sample.as_secs_f64() * 1000.0;
        Some(previous.map_or(sample, |p| p * 0.8 + sample * 0.2))
    }
    fn record_cost(&mut self, cost: Duration) {
        self.cost_ms = Self::ewma(self.cost_ms, cost);
    }
    fn record_interval(&mut self, interval: Duration) {
        self.interval_ms = Self::ewma(self.interval_ms, interval);
    }
    fn cost_ms(&self) -> f64 {
        self.cost_ms.unwrap_or(0.0)
    }
    fn fps(&self) -> f64 {
        self.interval_ms
            .filter(|i| *i > 0.0)
            .map_or(0.0, |i| 1000.0 / i)
    }
    fn next_interval(&self) -> Duration {
        Duration::from_secs_f64((self.cost_ms() * PACING_HEADROOM / 1000.0).max(1.0 / TARGET_FPS))
    }
}
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub struct Companion {
    pub shared: Arc<Mutex<Shared>>,
    pane: Arc<Mutex<Pane>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Companion {
    /// Start the renderer. Kitty/Ghostty receive PNG frames; iTerm2 and cell graphics receive JPEG.
    pub fn start(cfg: Config, graphics: Graphics) -> Self {
        Self::start_with(cfg, graphics.frame_format())
    }
    fn start_with(cfg: Config, format: FrameFormat) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: "Opening her room…".into(),
            motion: Motion {
                state: "idle".into(),
                mood: "neutral".into(),
                ..Default::default()
            },
            ..Default::default()
        }));
        let pane = Arc::new(Mutex::new(Pane::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let sh = shared.clone();
        let pn = pane.clone();
        let st = stop.clone();
        let handle = thread::spawn(move || {
            for attempt in 1..=2 {
                sh.lock().unwrap_or_else(|e| e.into_inner()).info["attempt"] = json!(attempt);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    render_loop(&cfg, &sh, &pn, &st, format)
                }));
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(format!("{e:#}")),
                    Err(_) => Some("The renderer worker stopped unexpectedly".into()),
                };
                if let Some(error) = error {
                    let phase = sh.lock().unwrap_or_else(|e| e.into_inner()).status.clone();
                    let retry = attempt == 1
                        && !st.load(Ordering::Relaxed)
                        && !crate::lifecycle::requested()
                        && sh.lock().unwrap_or_else(|e| e.into_inner()).frames == 0
                        && phase != "Finding her model…";
                    if retry {
                        sh.lock().unwrap_or_else(|e| e.into_inner()).info["first_startup_error"] =
                            json!(crate::tools::clip(&error, 2000));
                        report(
                            &cfg,
                            &sh,
                            "Restarting her renderer after a startup failure…",
                            Some(&phase),
                        );
                        thread::sleep(Duration::from_millis(300));
                        continue;
                    }
                    sh.lock().unwrap_or_else(|e| e.into_inner()).frame = None;
                    report(
                        &cfg,
                        &sh,
                        &format!("Live2D unavailable: {error}\n/pet retry · /status details"),
                        Some(&phase),
                    );
                }
                break;
            }
        });
        Self {
            shared,
            pane,
            stop,
            handle: Some(handle),
        }
    }
    pub fn motion(&self, state: &str, mood: &str, tap: bool, look: bool) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        s.motion.state = state.into();
        s.motion.mood = mood.into();
        s.motion.tap |= tap;
        s.motion.look |= look;
    }
    /// Pixel size of the portrait area. Cheap and idempotent: the renderer resizes and re-frames on
    /// its next frame. A zero side (unknown size) is ignored; other sizes are scaled into
    /// `VIEW_MIN..=VIEW_MAX` per side, keeping the aspect ratio where possible.
    pub fn set_view(&self, width_px: u32, height_px: u32) {
        if let Some(view) = clamp_view(width_px, height_px) {
            self.pane.lock().unwrap_or_else(|e| e.into_inner()).view = view;
        }
    }
    /// Queue a cosmetic control. A renderer acknowledgement reports actual acceptance.
    pub fn control(&self, command: crate::companion::Control) -> Result<String> {
        command.validate()?;
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.motion.commands.len() >= 32 {
            bail!("Companion control queue is full; wait for the renderer")
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        s.motion.commands.push(json!({"id":id,"command":command}));
        Ok(id)
    }
    pub fn current(&self) -> Shared {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
impl Drop for Companion {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
struct Browser {
    child: crate::lifecycle::ManagedChild,
    _profile: tempfile::TempDir,
}
impl Browser {
    fn check_running(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("Chrome exited ({status}). {}", self.diagnostic());
        }
        Ok(())
    }
    fn diagnostic(&self) -> String {
        let path = self._profile.path().join("renderer.log");
        let Ok(mut file) = fs::File::open(path) else {
            return String::new();
        };
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = file.seek(SeekFrom::Start(size.saturating_sub(4096)));
        let mut bytes = Vec::new();
        let _ = file.take(4096).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes)
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect()
    }
    fn wait_port(&mut self, stop: &AtomicBool, timeout: Duration) -> Result<Option<u16>> {
        let started = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) || crate::lifecycle::requested() {
                return Ok(None);
            }
            self.check_running()?;
            if let Ok(text) = fs::read_to_string(self._profile.path().join("DevToolsActivePort"))
                && let Some(port) = text.lines().next().and_then(|p| p.parse::<u16>().ok())
            {
                return Ok(Some(port));
            }
            if started.elapsed() > timeout {
                bail!("Chrome did not become ready. {}", self.diagnostic());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
struct AssetServer {
    url: String,
    /// Optional model references (motions, expressions, pose, user data) whose files are missing.
    unserved: Vec<String>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for AssetServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}
/// Files the Live2D runtime loads on demand: pose, user data, expressions, motions and their sounds.
fn optional_references(definition: &Value) -> Result<Vec<String>> {
    let refs = &definition["FileReferences"];
    let mut found: Vec<&str> = ["Pose", "UserData"]
        .iter()
        .filter_map(|key| refs[*key].as_str())
        .collect();
    for expression in refs["Expressions"].as_array().into_iter().flatten() {
        found.push(
            expression["File"]
                .as_str()
                .context("Invalid expression file")?,
        );
    }
    for group in refs["Motions"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.values())
    {
        for motion in group.as_array().context("Invalid motion group")? {
            found.push(motion["File"].as_str().context("Invalid motion file")?);
            found.extend(motion["Sound"].as_str());
        }
    }
    let mut out: Vec<String> = found.into_iter().map(String::from).collect();
    out.sort();
    out.dedup();
    Ok(out)
}
fn content_type(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    if name.ends_with(".js") {
        "application/javascript"
    } else if name.ends_with(".png") {
        "image/png"
    } else if name.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}
fn assets(cfg: &Config) -> Result<AssetServer> {
    let profile = crate::companion::Profile::load(cfg.companion_profile.as_deref())?;
    let model_file = cfg.pet.join(&profile.model);
    let model = model_file
        .parent()
        .context("Model has no parent directory")?;
    let root = cfg.pet.canonicalize()?;
    if !model_file
        .canonicalize()
        .context("Cannot find model; check --pet-dir and --companion-profile")?
        .starts_with(&root)
    {
        bail!("Live2D model escapes the asset directory");
    }
    let mut text = Vec::new();
    fs::File::open(&model_file)?
        .take(1_000_001)
        .read_to_end(&mut text)?;
    if text.len() > 1_000_000 {
        bail!("Model definition exceeds 1 MB")
    }
    let definition: Value = serde_json::from_slice(&text)?;
    let mut paths = HashMap::<String, PathBuf>::new();
    let filename = model_file
        .file_name()
        .context("Model has no file name")?
        .to_str()
        .context("Model name is not UTF-8")?;
    paths.insert(format!("model/{filename}"), model_file.clone());
    let mut reference = |p: &str| -> Result<()> {
        if !crate::companion::relative(p) {
            bail!("Live2D references must be relative local paths")
        }
        paths.insert(format!("model/{p}"), model.join(p));
        Ok(())
    };
    for key in ["Moc", "Physics", "DisplayInfo"] {
        if let Some(p) = definition["FileReferences"][key].as_str() {
            reference(p)?;
        }
    }
    for p in definition["FileReferences"]["Textures"]
        .as_array()
        .context("No Live2D textures")?
    {
        reference(p.as_str().context("Invalid texture name")?)?;
    }
    for name in [
        "pixi.min.js",
        "live2dcubismcore.min.js",
        "pixi-live2d-display-cubism4.min.js",
    ] {
        paths.insert(format!("vendor/{name}"), cfg.pet.join("vendor").join(name));
    }
    for path in paths.values() {
        if !path.canonicalize()?.starts_with(&root) {
            bail!("Live2D reference escapes the asset directory")
        }
    }
    // Optional files follow the same rules; a missing one is left unserved rather than failing startup.
    let mut unserved = Vec::new();
    for reference in optional_references(&definition)? {
        if !crate::companion::relative(&reference) {
            bail!("Live2D references must be relative local paths")
        }
        let path = model.join(&reference);
        match path.canonicalize() {
            Ok(real) if real.starts_with(&root) => {
                paths.insert(format!("model/{reference}"), path);
            }
            Ok(_) => bail!("Live2D reference escapes the asset directory"),
            Err(_) => unserved.push(reference),
        }
    }
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| anyhow::anyhow!("{e}"))?;
    let address = server
        .server_addr()
        .to_ip()
        .context("No renderer address")?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let prefix = format!("/{token}/");
    let url = format!("http://{address}{prefix}");
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let expected_host = address.to_string();
    let settings = json!({"texture_size":cfg.texture_size,"profile":profile})
        .to_string()
        .into_bytes();
    let join = thread::spawn(move || {
        while !s.load(Ordering::Relaxed) {
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) {
                let host = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Host"))
                    .map(|h| h.value.as_str());
                if request.method() != &tiny_http::Method::Get
                    || host != Some(expected_host.as_str())
                {
                    let _ = request.respond(tiny_http::Response::empty(403));
                    continue;
                }
                let decoded =
                    percent_encoding::percent_decode_str(request.url()).decode_utf8_lossy();
                let Some(name) = decoded.strip_prefix(&prefix) else {
                    let _ = request.respond(tiny_http::Response::empty(404));
                    continue;
                };
                let result = if name.is_empty() {
                    Some((
                        include_bytes!("../live2d/renderer.html").to_vec(),
                        "text/html",
                    ))
                } else if name == "companion.js" {
                    Some((
                        include_bytes!("../live2d/companion.js").to_vec(),
                        "application/javascript",
                    ))
                } else if name == "settings.json" {
                    Some((settings.clone(), "application/json"))
                } else {
                    paths
                        .get(name)
                        .and_then(|p| fs::read(p).ok())
                        .map(|b| (b, content_type(name)))
                };
                if let Some((body, mime)) = result {
                    let response = tiny_http::Response::from_data(body)
                        .with_header(tiny_http::Header::from_bytes("Content-Type", mime).unwrap())
                        .with_header(
                            tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap(),
                        );
                    let _ = request.respond(response);
                } else {
                    let _ = request.respond(tiny_http::Response::empty(404));
                }
            }
        }
    });
    Ok(AssetServer {
        url,
        unserved,
        stop,
        join: Some(join),
    })
}
struct Cdp {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    id: u64,
}
fn report(cfg: &Config, shared: &Arc<Mutex<Shared>>, status: &str, failed_phase: Option<&str>) {
    let record = {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.status = status.into();
        if let Some(phase) = failed_phase {
            s.info["failed_phase"] = json!(phase);
        }
        json!({"status":status,"info":s.info,"frames":s.frames,"pid":std::process::id(),"time":chrono::Utc::now().to_rfc3339()})
    };
    let dir = cfg.state.join("diagnostics");
    if fs::create_dir_all(&dir).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        let _ = crate::session::atomic_json(&dir.join("live2d.json"), &record);
    }
}
impl Cdp {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let id = self.id;
        self.socket.send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))?;
        loop {
            let msg = self.socket.read()?;
            if let Message::Text(text) = msg {
                let v: Value = serde_json::from_str(&text)?;
                if v["id"] == id {
                    if v.get("error").is_some() {
                        bail!("Renderer protocol error")
                    };
                    return Ok(v["result"].clone());
                }
            }
        }
    }
    fn evaluate(&mut self, expression: &str) -> Result<Value> {
        let v = self.call(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true}),
        )?;
        if v.get("exceptionDetails").is_some() {
            bail!("Live2D renderer script error")
        };
        Ok(v["result"]["value"].clone())
    }
}
fn render_loop(
    cfg: &Config,
    shared: &Arc<Mutex<Shared>>,
    pane: &Arc<Mutex<Pane>>,
    stop: &Arc<AtomicBool>,
    format: FrameFormat,
) -> Result<()> {
    report(cfg, shared, "Finding her model…", None);
    if !cfg.chrome.is_file() {
        bail!("Chrome/Chromium not found; set ASTER_CHROME")
    }
    let server = assets(cfg)?;
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if !server.unserved.is_empty() {
            s.info["unserved_references"] = json!(server.unserved);
        }
    }
    let profile = tempfile::Builder::new().prefix("aster-live2d-").tempdir()?;
    let log = fs::File::create(profile.path().join("renderer.log"))?;
    report(cfg, shared, "Starting the Live2D renderer…", None);
    let mut command = Command::new(&cfg.chrome);
    command
        .args([
            "--headless=new",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--disable-background-networking",
            "--no-proxy-server",
            "--metrics-recording-only",
            "--mute-audio",
            "--hide-scrollbars",
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=0",
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            "--window-size=420,620",
        ])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg(&server.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    let child = crate::lifecycle::spawn_group(&mut command)?;
    let mut browser = Browser {
        child,
        _profile: profile,
    };
    let started = Instant::now();
    let Some(port) = browser.wait_port(stop, Duration::from_secs(20))? else {
        return Ok(());
    };
    report(cfg, shared, "Connecting to her renderer…", None);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()?;
    let url = loop {
        browser.check_running()?;
        let tabs = client
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()?
            .json::<Vec<Value>>()?;
        if let Some(url) = tabs
            .iter()
            .find(|t| {
                t["type"] == "page"
                    && t["url"]
                        .as_str()
                        .is_some_and(|s| s.starts_with(&server.url))
            })
            .and_then(|t| t["webSocketDebuggerUrl"].as_str())
        {
            break url.to_string();
        }
        if started.elapsed() > Duration::from_secs(25) {
            bail!("No private renderer tab")
        };
        if stop.load(Ordering::Relaxed) || crate::lifecycle::requested() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    };
    let (mut socket, _) = tungstenite::connect(&url)?;
    if let MaybeTlsStream::Plain(tcp) = socket.get_mut() {
        tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    }
    let mut cdp = Cdp { socket, id: 0 };
    report(cfg, shared, "Loading her Live2D model…", None);
    let mut last_loading = Value::Null;
    loop {
        if stop.load(Ordering::Relaxed) || crate::lifecycle::requested() {
            return Ok(());
        }
        let value = cdp.evaluate(
            "({ready:window.asterReady,error:window.asterError,info:window.asterInfo,stage:window.asterStage,loading:window.asterLoading,document:document.readyState})",
        )?;
        if value["loading"].is_object() && value["loading"] != last_loading {
            last_loading = value["loading"].clone();
            shared.lock().unwrap().info["loading"] = last_loading.clone();
            report(
                cfg,
                shared,
                &format!(
                    "Loading her model · {}/{} textures",
                    last_loading["loaded"].as_u64().unwrap_or(0),
                    last_loading["requested"].as_u64().unwrap_or(0)
                ),
                None,
            );
        }
        if value["ready"] == true {
            {
                let mut s = shared.lock().unwrap();
                if let Some(info) = value["info"].as_object() {
                    for (key, value) in info {
                        s.info[key] = value.clone();
                    }
                }
            }
            report(cfg, shared, "Drawing her first frame…", None);
            break;
        }
        if let Some(e) = value["error"].as_str().filter(|e| !e.is_empty()) {
            bail!("{e}")
        }
        if started.elapsed() > Duration::from_secs(60) {
            bail!(
                "Model loading timed out ({value}). {}",
                browser.diagnostic()
            )
        };
        thread::sleep(Duration::from_millis(120));
    }
    let mut pacing = Pacing::default();
    let mut previous: Option<Instant> = None;
    let mut last_info: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) && !crate::lifecycle::requested() {
        let frame_start = Instant::now();
        let elapsed = previous.map(|p| frame_start.duration_since(p));
        if let Some(interval) = elapsed {
            pacing.record_interval(interval);
        }
        previous = Some(frame_start);
        let dt = elapsed
            .unwrap_or(Duration::from_secs_f64(1.0 / TARGET_FPS))
            .as_millis()
            .clamp(16, 250);
        let motion = {
            let mut s = shared.lock().unwrap();
            let m = s.motion.clone();
            s.motion.tap = false;
            s.motion.look = false;
            s.motion.commands.clear();
            m
        };
        let view = pane.lock().unwrap_or_else(|e| e.into_inner()).view;
        let input = json!({
            "state": motion.state,
            "mood": motion.mood,
            "tap": motion.tap,
            "look": motion.look,
            "commands": motion.commands,
            "view": [view.0, view.1],
            "format": format.page_name(),
        });
        let value = cdp.evaluate(&format!("window.asterFrame({input},{dt})"))?;
        let data = STANDARD.decode(
            value["data"]
                .as_str()
                .context("Renderer returned no frame")?,
        )?;
        if data.is_empty() {
            bail!("Renderer returned an empty frame")
        }
        let side = |key: &str| {
            value[key]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0)
        };
        let (width, height) = (side("width"), side("height"));
        pacing.record_cost(frame_start.elapsed());
        let frames = {
            let mut s = shared.lock().unwrap();
            s.frames += 1;
            let bytes = data.len();
            s.frame = Some(PortraitFrame::new(data, format, s.frames, width, height));
            s.fps = pacing.fps();
            s.info["control"] = value["control"].clone();
            if value["results"].as_array().is_some_and(|v| !v.is_empty()) {
                s.info["control_results"] = value["results"].clone();
            }
            for key in ["framing", "warnings"] {
                if let Some(v) = value.get(key) {
                    s.info[key] = v.clone();
                }
            }
            if last_info.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
                last_info = Some(Instant::now());
                s.info["frame_format"] = json!(format.page_name());
                s.info["frame_size"] = json!([width, height]);
                s.info["frame_bytes"] = json!(bytes);
                s.info["fps"] = json!(round1(pacing.fps()));
                s.info["frame_ms"] = json!(round1(pacing.cost_ms()));
                s.info["page_ms"] = value["ms"].clone();
            }
            s.frames
        };
        if frames == 1 || frames == FPS_REPORT_FRAME {
            report(cfg, shared, "Live2D · connected", None);
        }
        // Never busy-loop: always rest, and rest longer when Chromium needs longer per frame.
        let wake = frame_start + pacing.next_interval().max(frame_start.elapsed() + MIN_REST);
        while !stop.load(Ordering::Relaxed) && !crate::lifecycle::requested() {
            let left = wake.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            thread::sleep(left.min(Duration::from_millis(50)));
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Graphics {
    Iterm,
    Kitty,
    Halfblocks,
    Off,
}
impl Graphics {
    pub fn detect(choice: &str) -> Self {
        match choice {
            "iterm" => Self::Iterm,
            "kitty" => Self::Kitty,
            "halfblocks" => Self::Halfblocks,
            "off" => Self::Off,
            _ => {
                let program = std::env::var("TERM_PROGRAM")
                    .unwrap_or_default()
                    .to_lowercase();
                let term = std::env::var("TERM").unwrap_or_default();
                if program.contains("iterm") {
                    Self::Iterm
                } else if program.contains("ghostty")
                    || term.contains("kitty")
                    || std::env::var_os("KITTY_WINDOW_ID").is_some()
                {
                    Self::Kitty
                } else {
                    Self::Halfblocks
                }
            }
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Iterm => "iTerm images",
            Self::Kitty => "Kitty images",
            Self::Halfblocks => "cell graphics",
            Self::Off => "hidden",
        }
    }
    /// The frame encoding this protocol displays: PNG for Kitty's `f=100`, compact JPEG otherwise.
    pub fn frame_format(self) -> FrameFormat {
        if self == Self::Kitty {
            FrameFormat::Png
        } else {
            FrameFormat::Jpeg
        }
    }
    /// Terminal escape sequence drawing `frame` over `area`, or an empty string when this protocol
    /// cannot show it (cell graphics, hidden, or a JPEG frame for Kitty).
    pub fn encode(self, frame: &PortraitFrame, area: Rect) -> String {
        if area.width == 0 || area.height == 0 || frame.data.is_empty() {
            return String::new();
        }
        let place = format!("\x1b7\x1b[{};{}H", area.y + 1, area.x + 1);
        match self {
            Self::Iterm => format!(
                "{place}\x1b]1337;File=inline=1;size={};width={};height={};preserveAspectRatio=1:{}\x07\x1b8",
                frame.data.len(),
                area.width,
                area.height,
                STANDARD.encode(&frame.data)
            ),
            // One image id and placement id for every frame: Kitty replaces the pixels in place
            // once the final chunk arrives, so the portrait never blanks between frames.
            Self::Kitty if frame.format == FrameFormat::Png => {
                let data = STANDARD.encode(&frame.data);
                let mut s = place;
                for (i, chunk) in data.as_bytes().chunks(4096).enumerate() {
                    let more = usize::from((i + 1) * 4096 < data.len());
                    let params = if i == 0 {
                        format!(
                            "a=T,f=100,i={KITTY_IMAGE_ID},p=1,q=2,C=1,c={},r={},m={more}",
                            area.width, area.height
                        )
                    } else {
                        format!("m={more},q=2")
                    };
                    s += &format!(
                        "\x1b_G{params};{}\x1b\\",
                        std::str::from_utf8(chunk).unwrap()
                    );
                }
                s + "\x1b8"
            }
            _ => String::new(),
        }
    }
    pub fn clear(self) -> &'static str {
        if self == Self::Kitty {
            "\x1b_Ga=d,d=I,i=731,q=2\x1b\\"
        } else {
            ""
        }
    }
}

/// Block glyphs indexed by their foreground quadrants: bit 0 top-left, 1 top-right,
/// 2 bottom-left, 3 bottom-right.
const QUADRANT_GLYPHS: [&str; 16] = [
    " ", "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];
#[derive(Clone, Copy, Debug, PartialEq)]
struct Quadrant {
    glyph: &'static str,
    fg: [u8; 3],
    bg: [u8; 3],
}
fn mean_color(pixels: &[[u8; 3]; 4], pick: impl Fn(usize) -> bool) -> [u8; 3] {
    let mut sum = [0u32; 3];
    let mut n = 0u32;
    for (_, pixel) in pixels.iter().enumerate().filter(|(i, _)| pick(*i)) {
        for (total, channel) in sum.iter_mut().zip(pixel) {
            *total += u32::from(*channel);
        }
        n += 1;
    }
    if n == 0 {
        return [0; 3];
    }
    sum.map(|v| ((v + n / 2) / n) as u8)
}
fn color_error(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (i32::from(*x) - i32::from(y)).pow(2) as u32)
        .sum()
}
/// The two-colour split of a 2x2 pixel block with the least squared error.
fn quadrant(pixels: [[u8; 3]; 4]) -> Quadrant {
    let mut best = (u32::MAX, 0usize, [0u8; 3], [0u8; 3]);
    // Masks 0..8 visit every partition once: a mask and its complement split the block alike.
    for mask in 0..8usize {
        let on = mean_color(&pixels, |i| mask >> i & 1 == 1);
        let off = mean_color(&pixels, |i| mask >> i & 1 == 0);
        let error = pixels
            .iter()
            .enumerate()
            .map(|(i, p)| color_error(*p, if mask >> i & 1 == 1 { on } else { off }))
            .sum::<u32>();
        if error < best.0 {
            best = (error, mask, on, off);
        }
    }
    let (_, mask, on, off) = best;
    if mask == 0 {
        return Quadrant {
            glyph: " ",
            fg: off,
            bg: off,
        };
    }
    // Draw the smaller side of the split as the glyph.
    let (mask, fg, bg) = if mask.count_ones() == 3 {
        (!mask & 0xF, off, on)
    } else {
        (mask, on, off)
    };
    Quadrant {
        glyph: QUADRANT_GLYPHS[mask],
        fg,
        bg,
    }
}
fn quadrant_cells(rgb: &image::RgbImage, cols: u16, rows: u16) -> Vec<Quadrant> {
    let (width, height) = (u32::from(cols) * 2, u32::from(rows) * 2);
    let resized;
    let image = if rgb.dimensions() == (width, height) {
        rgb
    } else {
        resized =
            image::imageops::resize(rgb, width, height, image::imageops::FilterType::CatmullRom);
        &resized
    };
    let mut cells = Vec::with_capacity(usize::from(cols) * usize::from(rows));
    for row in 0..u32::from(rows) {
        for col in 0..u32::from(cols) {
            let at = |dx: u32, dy: u32| image.get_pixel(col * 2 + dx, row * 2 + dy).0;
            cells.push(quadrant([at(0, 0), at(1, 0), at(0, 1), at(1, 1)]));
        }
    }
    cells
}
/// Character-cell portrait: each cell shows a 2x2 block of the frame as a quadrant glyph in two
/// colours. The result is cached on the frame for its area size.
pub fn halfblocks(frame: &PortraitFrame, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(cells) = frame.quadrants(area.width, area.height) else {
        return;
    };
    let rgb = |c: [u8; 3]| Color::Rgb(c[0], c[1], c[2]);
    for (i, q) in cells.iter().enumerate() {
        let x = area.x + (i % usize::from(area.width)) as u16;
        let y = area.y + (i / usize::from(area.width)) as u16;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(q.glyph).set_fg(rgb(q.fg)).set_bg(rgb(q.bg));
        }
    }
}
pub fn probe(cfg: &Config, dir: &Path, emotion: Option<&str>, motion: Option<&str>) -> Result<()> {
    fs::create_dir_all(dir)?;
    let c = Companion::start_with(cfg.clone(), FrameFormat::Png);
    let started = Instant::now();
    let mut first = None;
    loop {
        if crate::lifecycle::requested() {
            bail!("Renderer probe interrupted");
        }
        let s = c.current();
        if s.status.starts_with("Live2D unavailable") {
            bail!(s.status)
        }
        if let Some(frame) = s.frame {
            if let Some((initial, sequence)) = &first {
                if frame.sequence < *sequence + 30 {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                fs::write(dir.join("speaking.png"), &frame.data)?;
                if &frame.data == initial {
                    bail!("Animation frames were identical")
                };
                fs::write(
                    dir.join("renderer.json"),
                    serde_json::to_vec_pretty(
                        &json!({"info":s.info,"frames":s.frames,"fps":round1(s.fps),"animation_changes":true}),
                    )?,
                )?;
                fs::write(
                    dir.join("interface.json"),
                    serde_json::to_vec_pretty(&s.info["api"])?,
                )?;
                if emotion.is_some() || motion.is_some() {
                    use crate::companion::Control;
                    c.motion("idle", "neutral", false, false);
                    let mut results = vec![];
                    let mut capture = |label: &str, command: Control| -> Result<()> {
                        let id = c.control(command)?;
                        let until = Instant::now() + Duration::from_secs(10);
                        loop {
                            if crate::lifecycle::requested() {
                                bail!("Preview interrupted")
                            }
                            let observed = c.current();
                            if observed.status.starts_with("Live2D unavailable") {
                                bail!(observed.status)
                            }
                            if let Some(result) = observed.info["control_results"]
                                .as_array()
                                .and_then(|rs| rs.iter().find(|r| r["id"] == id))
                            {
                                if result["ok"] != true {
                                    bail!("Control rejected: {}", result["error"])
                                }
                                fs::write(
                                    dir.join(format!("{label}.png")),
                                    &observed.frame.context("No preview frame")?.data,
                                )?;
                                results.push(json!({"label":label,"acknowledgement":result,"control":observed.info["control"]}));
                                return Ok(());
                            }
                            if Instant::now() >= until {
                                bail!("Companion control was not acknowledged")
                            }
                            thread::sleep(Duration::from_millis(20));
                        }
                    };
                    capture("neutral", Control::Reset)?;
                    if let Some(name) = emotion {
                        capture(
                            "emotion",
                            Control::Emotion {
                                name: name.into(),
                                strength: 0.7,
                            },
                        )?;
                    }
                    if let Some(name) = motion {
                        capture(
                            "motion",
                            Control::Motion {
                                name: name.into(),
                                strength: 0.8,
                            },
                        )?;
                    }
                    capture(
                        "look",
                        Control::Look {
                            x: 0.4,
                            y: 0.2,
                            duration_ms: 1000,
                        },
                    )?;
                    capture("reset", Control::Reset)?;
                    fs::write(
                        dir.join("controls.json"),
                        serde_json::to_vec_pretty(&results)?,
                    )?;
                }
                println!(
                    "Live2D model loaded; {} frames at {:.1} fps captured in {}",
                    s.frames,
                    s.fps,
                    dir.display()
                );
                return Ok(());
            } else {
                fs::write(dir.join("idle.png"), &frame.data)?;
                first = Some((frame.data, frame.sequence));
                c.motion("speaking", "happy", true, false);
            }
        }
        if started.elapsed() > Duration::from_secs(75) {
            bail!("Live2D probe timed out")
        };
        thread::sleep(Duration::from_millis(100));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn config(root: &Path, pet: PathBuf, chrome: PathBuf) -> Config {
        Config {
            home: root.into(),
            project: root.into(),
            state: root.join("state"),
            key: String::new(),
            base: String::new(),
            model: String::new(),
            pet,
            chrome,
            companion_profile: None,
            texture_size: 1024,
            limits: Default::default(),
            auth: Default::default(),
            provider: "MiniMax".into(),
        }
    }
    fn http() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }
    /// A pet directory with vendor placeholders and a model3.json with extra FileReferences.
    fn pet_fixture(root: &Path, references: Value) -> PathBuf {
        let pet = root.join("assets");
        let model = pet.join("弄玉运行档_无水印");
        fs::create_dir_all(&model).unwrap();
        fs::create_dir_all(pet.join("vendor")).unwrap();
        let mut definition =
            json!({"FileReferences":{"Moc":"model.moc3","Textures":["texture.png"]}});
        for (key, value) in references.as_object().unwrap() {
            definition["FileReferences"][key] = value.clone();
        }
        fs::write(
            model.join("弄玉.model3.json"),
            serde_json::to_vec(&definition).unwrap(),
        )
        .unwrap();
        for name in ["model.moc3", "texture.png"] {
            fs::write(model.join(name), "fixture").unwrap();
        }
        for name in [
            "pixi.min.js",
            "live2dcubismcore.min.js",
            "pixi-live2d-display-cubism4.min.js",
        ] {
            fs::write(pet.join("vendor").join(name), "fixture").unwrap();
        }
        pet
    }
    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    #[test]
    fn renderer_settings_are_local_and_asset_routes_stay_bounded() {
        let root = tempfile::tempdir().unwrap();
        let pet = pet_fixture(root.path(), json!({}));
        let cfg = config(root.path(), pet, root.path().join("chrome"));
        let server = assets(&cfg).unwrap();
        let client = http();
        let settings: Value = client
            .get(format!("{}settings.json", server.url))
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(settings["texture_size"], 1024);
        assert_eq!(settings["profile"]["version"], 1);
        assert!(
            client
                .get(format!("{}companion.js", server.url))
                .send()
                .unwrap()
                .text()
                .unwrap()
                .contains("AsterCompanion")
        );
        assert_eq!(
            client
                .get(format!("{}.env", server.url))
                .send()
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("{}settings.json", server.url))
                .header("Host", "untrusted.example")
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    #[test]
    fn model_json_references_are_served_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        let pet = pet_fixture(
            root.path(),
            json!({
                "Pose": "弄玉.pose3.json",
                "UserData": "弄玉.userdata3.json",
                "Expressions": [
                    {"Name": "Happy", "File": "expressions/happy.exp3.json"},
                    {"Name": "Gone", "File": "expressions/gone.exp3.json"}
                ],
                "Motions": {
                    "Idle": [{"File": "motions/idle.motion3.json", "Sound": "sounds/idle.wav"}],
                    "TapBody": [{"File": "motions/tap.motion3.json"}]
                }
            }),
        );
        let model = pet.join("弄玉运行档_无水印");
        let served = [
            "弄玉.pose3.json",
            "弄玉.userdata3.json",
            "expressions/happy.exp3.json",
            "motions/idle.motion3.json",
            "motions/tap.motion3.json",
        ];
        for name in served {
            write(&model.join(name), &format!("{{\"fixture\":\"{name}\"}}"));
        }
        write(&model.join("sounds/idle.wav"), "RIFF");
        write(&model.join("motions/unlisted.motion3.json"), "{}");
        let cfg = config(root.path(), pet.clone(), root.path().join("chrome"));
        let server = assets(&cfg).unwrap();
        assert_eq!(server.unserved, vec!["expressions/gone.exp3.json"]);
        let client = http();
        for name in served {
            let response = client
                .get(format!("{}model/{name}", server.url))
                .send()
                .unwrap();
            assert_eq!(response.status(), 200, "{name}");
            assert_eq!(
                response.headers()["content-type"],
                "application/json",
                "{name}"
            );
            let body: Value = response.json().unwrap();
            assert_eq!(body["fixture"], name);
        }
        // A motion's sound is served with it; files the model does not reference are not.
        let sound = client
            .get(format!("{}model/sounds/idle.wav", server.url))
            .send()
            .unwrap();
        assert_eq!(sound.status(), 200);
        assert_eq!(sound.text().unwrap(), "RIFF");
        for name in [
            "model/motions/unlisted.motion3.json",
            "model/expressions/gone.exp3.json",
            "model/弄玉运行档_无水印/弄玉.model3.json",
        ] {
            let status = client
                .get(format!("{}{name}", server.url))
                .send()
                .unwrap()
                .status();
            assert_eq!(status, 404, "{name}");
        }
        drop(server);

        // A reference that climbs out of the model folder refuses to start the server.
        let outside = tempfile::tempdir().unwrap();
        let pet = pet_fixture(
            outside.path(),
            json!({"Expressions": [{"Name": "Leak", "File": "../../secret.exp3.json"}]}),
        );
        write(&outside.path().join("secret.exp3.json"), "{}");
        let cfg = config(outside.path(), pet, outside.path().join("chrome"));
        let error = assets(&cfg).err().expect("escaping reference must fail");
        assert!(error.to_string().contains("relative local paths"));
        #[cfg(unix)]
        {
            let linked = tempfile::tempdir().unwrap();
            let pet = pet_fixture(
                linked.path(),
                json!({"Motions": {"Idle": [{"File": "motions/idle.motion3.json"}]}}),
            );
            write(&linked.path().join("private.json"), "{}");
            let motions = pet.join("弄玉运行档_无水印/motions");
            fs::create_dir_all(&motions).unwrap();
            std::os::unix::fs::symlink(
                linked.path().join("private.json"),
                motions.join("idle.motion3.json"),
            )
            .unwrap();
            let cfg = config(linked.path(), pet, linked.path().join("chrome"));
            assert!(assets(&cfg).is_err());
        }
    }
    fn encoded(image: &RgbImage, format: FrameFormat) -> PortraitFrame {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image.clone())
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                match format {
                    FrameFormat::Png => image::ImageFormat::Png,
                    FrameFormat::Jpeg => image::ImageFormat::Jpeg,
                },
            )
            .unwrap();
        PortraitFrame::new(bytes, format, 1, image.width(), image.height())
    }
    #[test]
    fn quadrant_cells_choose_the_least_error_glyph() {
        const W: [u8; 3] = [250, 250, 250];
        const K: [u8; 3] = [10, 12, 20];
        const R: [u8; 3] = [200, 30, 40];
        // Pixels are top-left, top-right, bottom-left, bottom-right.
        let cases = [
            ([W, K, K, K], "▘", W, K),
            ([K, W, K, K], "▝", W, K),
            ([K, K, W, K], "▖", W, K),
            ([K, K, K, W], "▗", W, K),
            ([W, W, W, K], "▗", K, W),
            ([W, W, K, K], "▀", W, K),
            ([K, K, W, W], "▀", K, W),
            ([W, K, W, K], "▌", W, K),
            ([K, W, K, W], "▌", K, W),
            ([K, W, W, K], "▞", W, K),
            ([W, K, K, W], "▞", K, W),
            ([W, W, W, W], " ", W, W),
            // Three colours: red+red+black against white has the least squared error.
            ([R, R, K, W], "▗", W, [137, 24, 33]),
        ];
        for (pixels, glyph, fg, bg) in cases {
            assert_eq!(quadrant(pixels), Quadrant { glyph, fg, bg }, "{pixels:?}");
        }
        // Through the public renderer: a 4x2 image fills two cells exactly.
        let mut image = RgbImage::from_pixel(4, 2, Rgb(K));
        image.put_pixel(0, 0, Rgb(W));
        image.put_pixel(2, 1, Rgb(W));
        image.put_pixel(3, 1, Rgb(W));
        let frame = encoded(&image, FrameFormat::Png);
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 3));
        halfblocks(&frame, Rect::new(2, 1, 2, 1), &mut buf);
        assert_eq!(buf[(2, 1)].symbol(), "▘");
        assert_eq!(buf[(2, 1)].fg, Color::Rgb(W[0], W[1], W[2]));
        assert_eq!(buf[(2, 1)].bg, Color::Rgb(K[0], K[1], K[2]));
        assert_eq!(buf[(3, 1)].symbol(), "▀");
        assert_eq!(buf[(3, 1)].fg, Color::Rgb(K[0], K[1], K[2]));
        assert_eq!(buf[(3, 1)].bg, Color::Rgb(W[0], W[1], W[2]));
        assert_eq!(buf[(1, 1)].symbol(), " ");
        // A larger frame is filtered down; the split survives and the result is cached per size.
        let mut large = RgbImage::from_pixel(40, 20, Rgb(K));
        for y in 0..20 {
            for x in 0..20 {
                large.put_pixel(x, y, Rgb(W));
            }
        }
        let frame = encoded(&large, FrameFormat::Png);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        halfblocks(&frame, Rect::new(0, 0, 1, 1), &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "▌");
        let Color::Rgb(r, _, _) = buf[(0, 0)].fg else {
            panic!("expected an RGB foreground")
        };
        assert!(r > 220, "{r}");
        let first = frame.quadrants(1, 1).unwrap();
        assert!(Arc::ptr_eq(&first, &frame.clone().quadrants(1, 1).unwrap()));
        assert!(!Arc::ptr_eq(&first, &frame.quadrants(2, 1).unwrap()));
    }
    #[test]
    fn frames_decode_lazily_once_for_all_clones() {
        let frame = encoded(
            &RgbImage::from_pixel(6, 4, Rgb([10, 20, 30])),
            FrameFormat::Jpeg,
        );
        assert!(frame.cache.rgb.get().is_none());
        let copy = frame.clone();
        let pixels = copy.rgb().unwrap();
        assert_eq!(pixels.dimensions(), (6, 4));
        assert!(Arc::ptr_eq(&pixels, &frame.rgb().unwrap()));
        assert!(
            PortraitFrame::new(b"not an image".to_vec(), FrameFormat::Png, 1, 1, 1)
                .rgb()
                .is_none()
        );
    }
    #[test]
    fn custom_model_profiles_allow_local_motion_and_expression_references_only() {
        let root = tempfile::tempdir().unwrap();
        let pet = root.path().join("assets");
        let model = pet.join("custom");
        fs::create_dir_all(&model).unwrap();
        fs::create_dir(pet.join("vendor")).unwrap();
        let metadata = model.join("avatar.model3.json");
        fs::write(&metadata, r#"{"FileReferences":{"Moc":"rig.moc3","Textures":["texture.png"],"Expressions":[{"Name":"smile","File":"smile.exp3.json"}],"Motions":{"Idle":[{"File":"idle.motion3.json"}]}}}"#).unwrap();
        for filename in [
            "rig.moc3",
            "texture.png",
            "smile.exp3.json",
            "idle.motion3.json",
        ] {
            fs::write(model.join(filename), "fixture").unwrap();
        }
        for filename in [
            "pixi.min.js",
            "live2dcubismcore.min.js",
            "pixi-live2d-display-cubism4.min.js",
        ] {
            fs::write(pet.join("vendor").join(filename), "fixture").unwrap();
        }
        let mut profile = crate::companion::Profile::load(None).unwrap();
        profile.model = "custom/avatar.model3.json".into();
        let path = root.path().join("profile.json");
        fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let mut cfg = config(root.path(), pet, root.path().join("chrome"));
        cfg.companion_profile = Some(path);
        let server = assets(&cfg).unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        for filename in ["smile.exp3.json", "idle.motion3.json"] {
            assert_eq!(
                client
                    .get(format!("{}model/{filename}", server.url))
                    .send()
                    .unwrap()
                    .text()
                    .unwrap(),
                "fixture"
            );
        }
        drop(server);
        fs::write(&metadata, r#"{"FileReferences":{"Moc":"rig.moc3","Textures":["texture.png"],"Motions":{"Idle":[{"File":"../secret.motion3.json"}]}}}"#).unwrap();
        assert!(assets(&cfg).is_err());
    }
    #[test]
    fn graphics_packets_match_frame_formats() {
        let a = Rect::new(2, 3, 20, 30);
        let png = PortraitFrame::new(vec![0; 9000], FrameFormat::Png, 1, 10, 10);
        let jpeg = PortraitFrame::new(b"jpeg-bytes".to_vec(), FrameFormat::Jpeg, 2, 10, 10);
        let it = Graphics::Iterm.encode(&jpeg, a);
        assert_eq!(
            it,
            format!(
                "\x1b7\x1b[4;3H\x1b]1337;File=inline=1;size=10;width=20;height=30;preserveAspectRatio=1:{}\x07\x1b8",
                STANDARD.encode(b"jpeg-bytes")
            )
        );
        assert!(Graphics::Iterm.encode(&png, a).contains(";size=9000;"));
        // Kitty's f=100 is PNG only: a JPEG frame is never sent to it.
        assert_eq!(Graphics::Kitty.encode(&jpeg, a), "");
        let kitty = Graphics::Kitty.encode(&png, a);
        assert!(
            kitty.starts_with("\x1b7\x1b[4;3H\x1b_Ga=T,f=100,i=731,p=1,q=2,C=1,c=20,r=30,m=1;")
        );
        assert_eq!(kitty.matches("\x1b_G").count(), 3);
        assert!(kitty.contains("\x1b_Gm=0,q=2;"));
        assert!(kitty.ends_with("\x1b\\\x1b8"));
        for graphics in [Graphics::Halfblocks, Graphics::Off] {
            assert_eq!(graphics.encode(&png, a), "");
        }
        assert_eq!(Graphics::Iterm.encode(&png, Rect::new(0, 0, 0, 5)), "");
        assert_eq!(Graphics::Kitty.frame_format(), FrameFormat::Png);
        for graphics in [Graphics::Iterm, Graphics::Halfblocks, Graphics::Off] {
            assert_eq!(graphics.frame_format(), FrameFormat::Jpeg);
        }
    }
    #[test]
    fn view_sizes_are_clamped_keeping_their_shape() {
        assert_eq!(clamp_view(420, 620), Some((420, 620)));
        assert_eq!(clamp_view(2000, 1000), Some((1400, 700)));
        assert_eq!(clamp_view(100, 50), Some((320, 160)));
        assert_eq!(clamp_view(5000, 100), Some((1400, 160)));
        assert_eq!(clamp_view(0, 600), None);
        let root = tempfile::tempdir().unwrap();
        let cfg = config(
            root.path(),
            root.path().join("pet"),
            root.path().join("missing-chrome"),
        );
        let companion = Companion::start(cfg, Graphics::Iterm);
        let view = || companion.pane.lock().unwrap().view;
        assert_eq!(view(), DEFAULT_VIEW);
        companion.set_view(3000, 3000);
        assert_eq!(view(), (1400, 1400));
        companion.set_view(0, 0);
        assert_eq!(view(), (1400, 1400));
        companion.set_view(300, 200);
        companion.set_view(300, 200);
        assert_eq!(view(), (300, 200));
    }
    #[test]
    fn pacing_targets_fifteen_fps_and_backs_off_when_slow() {
        let fifteen = Duration::from_secs_f64(1.0 / 15.0);
        let mut pacing = Pacing::default();
        assert_eq!(pacing.next_interval(), fifteen);
        pacing.record_cost(Duration::from_millis(20));
        assert_eq!(pacing.next_interval(), fifteen);
        let mut slow = Pacing::default();
        slow.record_cost(Duration::from_millis(100));
        assert_eq!(slow.next_interval(), Duration::from_millis(150));
        pacing.record_interval(Duration::from_millis(50));
        pacing.record_interval(Duration::from_millis(100));
        assert!((pacing.fps() - 1000.0 / 60.0).abs() < 1e-9);
    }
    #[test]
    #[cfg(unix)]
    fn browser_exit_reports_stderr_without_waiting_for_startup_timeout() {
        let profile = tempfile::tempdir().unwrap();
        let log = fs::File::create(profile.path().join("renderer.log")).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf 'renderer fixture failed' >&2; exit 7"])
            .stderr(log);
        let child = crate::lifecycle::spawn_group(&mut command).unwrap();
        let mut browser = Browser {
            child,
            _profile: profile,
        };
        let start = Instant::now();
        let error = browser
            .wait_port(&AtomicBool::new(false), Duration::from_secs(20))
            .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(error.to_string().contains("renderer fixture failed"));
        assert!(error.to_string().contains('7'));
    }
    #[test]
    fn startup_failure_replaces_loading_and_saves_diagnostics() {
        let root = tempfile::tempdir().unwrap();
        let cfg = Config {
            key: "must-not-appear-in-log".into(),
            texture_size: 2048,
            ..config(
                root.path(),
                root.path().join("pet"),
                root.path().join("missing-chrome"),
            )
        };
        let companion = Companion::start(cfg.clone(), Graphics::Kitty);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !companion.handle.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let state = companion.current();
        assert!(state.status.contains("Chrome/Chromium not found"));
        assert!(state.status.contains("/pet retry"));
        assert_eq!(state.info["failed_phase"], "Finding her model…");
        let log = fs::read_to_string(cfg.state.join("diagnostics/live2d.json")).unwrap();
        assert!(log.contains("Chrome/Chromium not found"));
        assert!(!log.contains(&cfg.key));
    }

    // ---- Headless end-to-end check with a stub rig -------------------------------------------
    //
    // Runs renderer.html and companion.js in a real Chromium through the real pipeline (asset
    // server, CDP, pacing, frame decoding) with tests/fixtures/live2d-stub.js standing in for Pixi
    // and the Live2D SDK. The stub draws the rendered parameter values into each PNG frame, so the
    // effect of every control can be measured, not just its acknowledgement.
    //   ASTER_TEST_CHROME=/path/to/chrome cargo test stub_renderer -- --ignored --nocapture

    /// Stub parameters in the order the fixture draws them: id, min, max.
    const STUB_PARAMS: [(&str, f64, f64); 18] = [
        ("ParamAngleX", -30.0, 30.0),
        ("ParamAngleY", -30.0, 30.0),
        ("ParamAngleZ", -30.0, 30.0),
        ("ParamBodyAngleX", -10.0, 10.0),
        ("ParamEyeBallX", -1.0, 1.0),
        ("ParamEyeBallY", -1.0, 1.0),
        ("ParamEyeLOpen", 0.0, 1.0),
        ("ParamEyeROpen", 0.0, 1.0),
        ("ParamBreath", 0.0, 1.0),
        ("ParamMouthOpenY", 0.0, 1.0),
        ("ParamEyeSmile", 0.0, 1.0),
        ("ParamEyeSmileR", 0.0, 1.0),
        ("ParamHeartEye", 0.0, 1.0),
        ("ParamTeers3", 0.0, 1.0),
        ("ParamTeers5", 0.0, 1.0),
        ("ParamTeers2", 0.0, 1.0),
        ("ParamMouthForm", -1.0, 1.0),
        ("ParamHairFront", -1.0, 1.0),
    ];
    struct Sample {
        phase: &'static str,
        sequence: u64,
        at: Instant,
        values: HashMap<&'static str, f64>,
        size: (u32, u32),
    }
    fn test_chrome() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("ASTER_TEST_CHROME") {
            return Some(path.into());
        }
        let mut found: Vec<PathBuf> = fs::read_dir("/opt/pw-browsers")
            .ok()?
            .flatten()
            .map(|e| e.path().join("chrome-linux/chrome"))
            .filter(|p| p.is_file())
            .collect();
        found.sort();
        found.pop()
    }
    /// A pet directory laid out like the real one (the default profile's model path), holding the
    /// stub in place of the vendor SDK and a model definition with an idle motion.
    fn stub_pet(root: &Path, chrome: &Path) -> (PathBuf, PathBuf) {
        let pet = root.join("pet");
        let model = pet.join("弄玉运行档_无水印");
        write(
            &pet.join("vendor/pixi.min.js"),
            include_str!("../tests/fixtures/live2d-stub.js"),
        );
        write(
            &pet.join("vendor/live2dcubismcore.min.js"),
            "/* test double: the stub needs no Cubism Core */",
        );
        write(
            &pet.join("vendor/pixi-live2d-display-cubism4.min.js"),
            "/* test double: defined in pixi.min.js */",
        );
        let definition = json!({
            "Version": 3,
            "FileReferences": {
                "Moc": "stub.moc3",
                "Textures": ["stub.4096/texture_00.png"],
                "Physics": "stub.physics3.json",
                "Motions": {"Idle": [{"File": "motions/idle.motion3.json"}]}
            }
        });
        write(&model.join("弄玉.model3.json"), &definition.to_string());
        write(&model.join("stub.moc3"), "stub");
        write(&model.join("stub.physics3.json"), "{}");
        write(
            &model.join("motions/idle.motion3.json"),
            &json!({"Version": 3, "Meta": {"Duration": 1.5}, "StubAmplitude": 6}).to_string(),
        );
        let texture = model.join("stub.4096/texture_00.png");
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        RgbImage::from_pixel(640, 640, Rgb([200, 180, 170]))
            .save(&texture)
            .unwrap();
        // Chromium refuses to run as root without --no-sandbox; this wrapper is test-only.
        let wrapper = root.join("chrome.sh");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec '{}' --no-sandbox \"$@\"\n",
                chrome.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        }
        (pet, wrapper)
    }
    fn decode_stub(frame: &PortraitFrame, phase: &'static str) -> Sample {
        let rgb = frame.rgb().expect("stub frames decode");
        let values = STUB_PARAMS
            .iter()
            .enumerate()
            .map(|(k, (id, min, max))| {
                let p = rgb.get_pixel(k as u32 * 4 + 1, 1).0;
                assert_eq!(p[2], 90, "parameter strip at {id}");
                let n = (u32::from(p[0]) << 8) | u32::from(p[1]);
                (*id, min + f64::from(n) / 65535.0 * (max - min))
            })
            .collect();
        Sample {
            phase,
            sequence: frame.sequence,
            at: Instant::now(),
            values,
            size: (frame.width, frame.height),
        }
    }
    fn record(companion: &Companion, phase: &'static str, seconds: f64, samples: &mut Vec<Sample>) {
        let started = Instant::now();
        let mut last = samples.last().map_or(0, |s| s.sequence);
        while started.elapsed().as_secs_f64() < seconds {
            let state = companion.current();
            assert!(
                !state.status.starts_with("Live2D unavailable"),
                "{}",
                state.status
            );
            if let Some(frame) = state.frame
                && frame.sequence != last
            {
                last = frame.sequence;
                samples.push(decode_stub(&frame, phase));
            }
            thread::sleep(Duration::from_millis(3));
        }
    }
    fn wait_for_frames(companion: &Companion, count: u64) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let state = companion.current();
            assert!(
                !state.status.starts_with("Live2D unavailable"),
                "{}",
                state.status
            );
            if state.frames >= count {
                return;
            }
            assert!(Instant::now() < deadline, "no frames: {}", state.status);
            thread::sleep(Duration::from_millis(20));
        }
    }
    /// The renderer's acknowledgement of one queued control.
    fn receipt(companion: &Companion, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = companion.current();
            if let Some(found) = state.info["control_results"]
                .as_array()
                .and_then(|rs| rs.iter().find(|r| r["id"] == id))
            {
                return found.clone();
            }
            assert!(Instant::now() < deadline, "no receipt for {id}");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn phase<'a>(samples: &'a [Sample], name: &str) -> Vec<&'a Sample> {
        samples.iter().filter(|s| s.phase == name).collect()
    }
    /// Largest change of `id` between consecutive frames within `phases`.
    fn max_step(samples: &[Sample], phases: &[&str], id: &str) -> f64 {
        samples
            .windows(2)
            .filter(|w| w[1].sequence == w[0].sequence + 1)
            .filter(|w| phases.contains(&w[0].phase) && phases.contains(&w[1].phase))
            .map(|w| (w[1].values[id] - w[0].values[id]).abs())
            .fold(0.0, f64::max)
    }
    fn fps_of(samples: &[Sample], name: &str) -> f64 {
        let s = phase(samples, name);
        let (first, last) = (s[0], s[s.len() - 1]);
        (last.sequence - first.sequence) as f64 / last.at.duration_since(first.at).as_secs_f64()
    }
    fn r3(v: f64) -> f64 {
        (v * 1000.0).round() / 1000.0
    }
    #[test]
    #[ignore = "needs Chromium: set ASTER_TEST_CHROME or install /opt/pw-browsers"]
    fn stub_renderer_applies_profile_controls_through_the_real_pipeline() {
        use crate::companion::Control;
        let chrome = test_chrome().expect("set ASTER_TEST_CHROME to a Chromium executable");
        let root = tempfile::tempdir().unwrap();
        let (pet, wrapper) = stub_pet(root.path(), &chrome);
        let cfg = config(root.path(), pet, wrapper);
        let companion = Companion::start_with(cfg.clone(), FrameFormat::Png);
        wait_for_frames(&companion, 3);
        let info = companion.current().info;
        assert_eq!(info["api"]["profile"], "弄玉", "{info:#}");
        assert_eq!(info["api"]["warnings"], json!([]), "every binding exists");
        assert_eq!(info["texture_sizes"][0]["rendered"], json!([640, 640]));

        // 1. Idle: companion.js owns the bound channels, so she sways gently and the SDK's own
        //    fast breathing wobble and once-a-second blink in the stub never show.
        let mut samples = Vec::new();
        record(&companion, "idle", 2.0, &mut samples);
        let idle = phase(&samples, "idle");
        let resting = idle
            .iter()
            .map(|s| s.values["ParamAngleY"].abs())
            .fold(0.0, f64::max);
        let sway = idle
            .iter()
            .map(|s| s.values["ParamAngleX"])
            .fold(f64::MIN, f64::max)
            - idle
                .iter()
                .map(|s| s.values["ParamAngleX"])
                .fold(f64::MAX, f64::min);
        assert!(sway > 0.05, "she is never still: {sway}");
        let idle_step = max_step(&samples, &["idle"], "ParamAngleX");
        assert!(idle_step < 1.0, "smooth: {idle_step}");

        // 2. An emotion with a strength. The UI's mood follows a frame after the command, as it
        //    does for /emotion; the strength must survive that.
        let id = companion
            .control(Control::Emotion {
                name: "happy".into(),
                strength: 0.5,
            })
            .unwrap();
        let ack = receipt(&companion, &id);
        assert_eq!(ack["ok"], true, "{ack}");
        assert_eq!(
            ack["result"]["supported_channels"],
            json!(["smile_l", "smile_r"])
        );
        companion.motion("idle", "happy", false, false);
        record(&companion, "happy", 0.8, &mut samples);
        let state = companion.current();
        assert_eq!(state.info["control"]["emotion"], "happy");
        assert_eq!(state.info["control"]["strength"], 0.5, "mood kept strength");
        assert!(
            state.info["control_results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["id"] != "mood"),
            "{}",
            state.info["control_results"]
        );
        let smile = phase(&samples, "happy").last().unwrap().values.clone();
        for id in ["ParamEyeSmile", "ParamEyeSmileR"] {
            assert!((smile[id] - 0.225).abs() < 0.002, "{id} {}", smile[id]);
        }

        // 3. A motion plays once, moves her head, and ends.
        let id = companion
            .control(Control::Motion {
                name: "nod".into(),
                strength: 1.0,
            })
            .unwrap();
        let ack = receipt(&companion, &id);
        assert_eq!(ack["ok"], true, "{ack}");
        assert_eq!(ack["result"]["duration_ms"], 1100);
        record(&companion, "nod", 1.6, &mut samples);
        let nodded = phase(&samples, "nod")
            .iter()
            .map(|s| s.values["ParamAngleY"])
            .fold(f64::MIN, f64::max);
        assert!(nodded > resting + 1.5, "nod {nodded} over {resting}");
        assert!(companion.current().info["control"]["motion"].is_null());

        // 4. Names the profile does not define are refused by the renderer, with the reason.
        let id = companion
            .control(Control::Emotion {
                name: "missing".into(),
                strength: 1.0,
            })
            .unwrap();
        let ack = receipt(&companion, &id);
        assert_eq!(ack["ok"], false);
        assert!(ack["error"].as_str().unwrap().contains("Unknown emotion"));

        // 5. Reset clears the emotion; the UI's mood follows.
        let id = companion.control(Control::Reset).unwrap();
        assert_eq!(receipt(&companion, &id)["ok"], true);
        companion.motion("idle", "neutral", false, false);
        record(&companion, "reset", 0.5, &mut samples);
        let reset = phase(&samples, "reset").last().unwrap().values["ParamEyeSmile"];
        assert!(reset.abs() < 1e-3, "{reset}");
        assert_eq!(companion.current().info["control"]["emotion"], "neutral");

        // 6. The canvas follows the pane and keeps the profile's layout.
        companion.set_view(300, 500);
        record(&companion, "resized", 1.0, &mut samples);
        assert_eq!(samples.last().unwrap().size, (300, 500));
        let framing = companion.current().info["framing"].clone();
        assert_eq!(framing["width"], 300, "{framing}");
        assert_eq!(framing["zoom"], 2.7);

        // Parameters the profile does not bind are never touched.
        for s in &samples {
            assert!(s.values["ParamMouthForm"].abs() < 1e-4);
            assert!(s.values["ParamHairFront"].abs() < 1e-4);
        }
        let state = companion.current();
        let fps = state.info["fps"].as_f64().unwrap();
        assert!(fps >= 8.0, "{fps}");
        let mut summary = json!({
            "fps_reported": fps,
            "fps_idle": r3(fps_of(&samples, "idle")),
            "frame_ms": state.info["frame_ms"],
            "page_ms": state.info["page_ms"],
            "frames_sampled": samples.len(),
            "idle_sway_deg": r3(sway),
            "idle_max_step_deg": r3(idle_step),
            "happy_0_5_smile": r3(smile["ParamEyeSmile"]),
            "nod_peak_deg": r3(nodded),
            "warnings": state.info["warnings"],
        });
        drop(companion);

        // The JPEG path used by iTerm2 and cell graphics.
        let companion = Companion::start(cfg, Graphics::Iterm);
        companion.set_view(640, 940);
        wait_for_frames(&companion, 40);
        let state = companion.current();
        let frame = state.frame.unwrap();
        assert_eq!(frame.format, FrameFormat::Jpeg);
        assert_eq!(&frame.data[..2], &[0xFF, 0xD8]);
        assert_eq!((frame.width, frame.height), (640, 940));
        let cells = Instant::now();
        let mut buf = Buffer::empty(Rect::new(0, 0, 46, 34));
        halfblocks(&frame, Rect::new(0, 0, 46, 34), &mut buf);
        let cells_ms = cells.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(Graphics::Kitty.encode(&frame, Rect::new(0, 0, 40, 30)), "");
        summary["jpeg_640x940"] = json!({
            "fps": state.info["fps"],
            "frame_ms": state.info["frame_ms"],
            "bytes": state.info["frame_bytes"],
            "cells_46x34_first_draw_ms": round1(cells_ms),
        });
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    }
}
