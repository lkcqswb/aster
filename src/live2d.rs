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
/// Time constant (seconds) of the characters-per-second measure behind her speaking motion.
const SPEECH_TAU: f64 = 0.35;
/// Below this many characters per second she has stopped speaking.
const SPEECH_FLOOR: f64 = 4.0;
const SPEECH_CAP: f64 = 120.0;
/// Speech rate for the speaking state until the UI reports streamed text through `speak`.
const STATE_SPEECH: f64 = 28.0;
const KITTY_IMAGE_ID: u32 = 731;

/// Emotions every rig understands: a procedural pose, plus whatever toggles, expression or
/// motion the rig provides for them. `emote` and `motion`'s mood also accept expression names.
pub const EMOTIONS: &[&str] = &[
    "neutral",
    "happy",
    "laugh",
    "shy",
    "love",
    "excited",
    "surprised",
    "confused",
    "thinking",
    "sad",
    "cry",
    "angry",
    "pout",
    "embarrassed",
    "sleepy",
    "proud",
    "worried",
    "dizzy",
];
/// One-shot gestures: a matching motion group, else arm/hand parameters, else a head/body version.
pub const GESTURES: &[&str] = &[
    "wave",
    "nod",
    "shake",
    "tilt",
    "think",
    "cheer",
    "heart",
    "cover",
    "stretch",
    "look_around",
    "fidget",
    "bow",
];
/// Longest emotion, gesture or expression name accepted from the UI or the override file.
const NAME_LIMIT: usize = 64;
/// Gestures waiting to be sent to the renderer; older ones are dropped beyond this.
const ACT_QUEUE: usize = 8;
/// A transient emotion lasts at most this long.
const EMOTE_MAX_SECONDS: f32 = 3600.0;
/// Size limit of the optional emotion/gesture override file.
const EMOTION_MAP_LIMIT: u64 = 64 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Motion {
    pub state: String,
    pub mood: String,
    pub tap: bool,
    pub look: bool,
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

/// Recent characters per second of streamed reply text, decaying exponentially between deltas.
#[derive(Clone, Copy, Debug, Default)]
struct SpeechMeter {
    rate: f64,
    at: Option<Instant>,
    heard: bool,
}
impl SpeechMeter {
    fn decay(&mut self, now: Instant) {
        if let Some(at) = self.at {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            self.rate *= (-dt / SPEECH_TAU).exp();
        }
        self.at = Some(now);
    }
    fn add(&mut self, chars: usize, now: Instant) {
        if chars == 0 {
            return;
        }
        self.decay(now);
        self.rate = (self.rate + chars as f64 / SPEECH_TAU).min(SPEECH_CAP);
        self.heard = true;
    }
    fn level(&mut self, now: Instant) -> f64 {
        self.decay(now);
        if self.rate < SPEECH_FLOOR {
            0.0
        } else {
            self.rate
        }
    }
}

/// What the UI tells the render thread besides the motion state.
#[derive(Default)]
struct Control {
    view: (u32, u32),
    speech: SpeechMeter,
    /// Transient emotion and when it ends.
    emotion: Option<(String, Instant)>,
    /// One-shot gestures or motion group names, sent one per frame.
    acts: std::collections::VecDeque<String>,
}
impl Control {
    fn new() -> Self {
        Self {
            view: DEFAULT_VIEW,
            ..Self::default()
        }
    }
    fn speech_for(&mut self, state: &str, now: Instant) -> (f64, &'static str) {
        let level = self.speech.level(now);
        if !self.speech.heard && state == "speaking" {
            (STATE_SPEECH, "state")
        } else {
            (level, "text")
        }
    }
    fn emote(&mut self, name: &str, seconds: f32, now: Instant) {
        self.emotion = match clean_name(name) {
            Some(name) if seconds.is_finite() && seconds > 0.0 => Some((
                name,
                now + Duration::from_secs_f32(seconds.min(EMOTE_MAX_SECONDS)),
            )),
            _ => None,
        };
    }
    /// The transient emotion still in effect at `now`, or "" once it has expired.
    fn transient(&mut self, now: Instant) -> String {
        if self
            .emotion
            .as_ref()
            .is_some_and(|(_, until)| *until <= now)
        {
            self.emotion = None;
        }
        self.emotion
            .as_ref()
            .map(|(name, _)| name.clone())
            .unwrap_or_default()
    }
    fn act(&mut self, name: &str) {
        if let Some(name) = clean_name(name) {
            self.acts.push_back(name);
            while self.acts.len() > ACT_QUEUE {
                self.acts.pop_front();
            }
        }
    }
}
/// A trimmed name of 1..=NAME_LIMIT characters without control characters.
fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= NAME_LIMIT && !name.chars().any(char::is_control))
        .then(|| name.to_string())
}

/// Validate the user's emotion/gesture override document into the shape the renderer reads:
/// `{"emotions":{name:{params:{id:number|null},expression,motion}},
///   "gestures":{name:{params:{id:number|[numbers]|null},motion}}}`.
/// Anything else is dropped with a warning.
fn sanitize_emotion_map(raw: &Value) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(top) = raw.as_object() else {
        return (Value::Null, vec!["expected a JSON object".into()]);
    };
    let mut out = json!({"emotions": {}, "gestures": {}});
    for (section, entries) in top {
        if section != "emotions" && section != "gestures" {
            warnings.push(format!("unknown key `{section}` ignored"));
            continue;
        }
        let gestures = section == "gestures";
        let Some(entries) = entries.as_object() else {
            warnings.push(format!("`{section}` must be an object"));
            continue;
        };
        for (name, entry) in entries.iter().take(64) {
            let Some(key) = clean_name(name).map(|n| n.to_lowercase()) else {
                warnings.push(format!("`{section}` has an invalid name"));
                continue;
            };
            let Some(fields) = entry.as_object() else {
                warnings.push(format!("`{section}.{key}` must be an object"));
                continue;
            };
            let mut clean = serde_json::Map::new();
            for (field, value) in fields {
                let at = format!("{section}.{key}.{field}");
                match field.as_str() {
                    "params" => {
                        let Some(params) = value.as_object() else {
                            warnings.push(format!("`{at}` must be an object"));
                            continue;
                        };
                        let mut kept = serde_json::Map::new();
                        for (id, v) in params.iter().take(32) {
                            let number = |v: &Value| v.as_f64().filter(|n| n.is_finite());
                            let ok = match v {
                                Value::Null => true,
                                Value::Number(_) => number(v).is_some(),
                                Value::Array(list) if gestures => {
                                    (1..=8).contains(&list.len())
                                        && list.iter().all(|n| number(n).is_some())
                                }
                                _ => false,
                            };
                            match clean_name(id) {
                                Some(id) if ok => {
                                    kept.insert(id, v.clone());
                                }
                                _ => warnings.push(format!(
                                    "`{at}.{}` needs {}",
                                    id.chars().take(NAME_LIMIT).collect::<String>(),
                                    if gestures {
                                        "a number, a list of up to 8 numbers, or null"
                                    } else {
                                        "a number or null"
                                    }
                                )),
                            }
                        }
                        clean.insert("params".into(), Value::Object(kept));
                    }
                    "motion" | "expression" if field == "motion" || !gestures => match value {
                        Value::Null => {
                            clean.insert(field.clone(), Value::Null);
                        }
                        Value::String(s) if clean_name(s).is_some() => {
                            clean.insert(field.clone(), json!(s.trim()));
                        }
                        _ => warnings.push(format!("`{at}` needs a name or null")),
                    },
                    _ => warnings.push(format!("unknown key `{at}` ignored")),
                }
            }
            out[section][key] = Value::Object(clean);
        }
    }
    (out, warnings)
}
/// Where the override file may live: beside the model assets, then in the user's config.
fn emotion_map_paths(pet: &Path) -> Vec<PathBuf> {
    let mut paths = vec![pet.join("aster-nongyu.json")];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".config/aster/nongyu.json"));
    }
    paths
}
/// Load the first override file found: (validated map or null, diagnostic record or null).
fn load_emotion_map(paths: &[PathBuf]) -> (Value, Value) {
    let Some(path) = paths.iter().find(|p| p.exists()) else {
        return (Value::Null, Value::Null);
    };
    let source = path.display().to_string();
    let failed = |error: String| (Value::Null, json!({"source": source, "error": error}));
    let file = match fs::File::open(path) {
        Ok(file) if file.metadata().is_ok_and(|m| m.is_file()) => file,
        Ok(_) => return failed("not a regular file".into()),
        Err(e) => return failed(format!("unreadable: {e}")),
    };
    let mut bytes = Vec::new();
    if let Err(e) = file.take(EMOTION_MAP_LIMIT + 1).read_to_end(&mut bytes) {
        return failed(format!("unreadable: {e}"));
    }
    if bytes.len() as u64 > EMOTION_MAP_LIMIT {
        return failed("larger than 64 KB".into());
    }
    let raw: Value = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        Err(e) => return failed(format!("invalid JSON: {e}")),
    };
    let (map, warnings) = sanitize_emotion_map(&raw);
    (map, json!({"source": source, "warnings": warnings}))
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
    control: Arc<Mutex<Control>>,
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
        let control = Arc::new(Mutex::new(Control::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let sh = shared.clone();
        let ctl = control.clone();
        let st = stop.clone();
        let handle = thread::spawn(move || {
            for attempt in 1..=2 {
                sh.lock().unwrap_or_else(|e| e.into_inner()).info["attempt"] = json!(attempt);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    render_loop(&cfg, &sh, &ctl, &st, format)
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
            control,
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
            self.control.lock().unwrap_or_else(|e| e.into_inner()).view = view;
        }
    }
    /// Report one streamed delta of reply text by its character count. Her mouth follows the
    /// recent characters-per-second and closes smoothly shortly after the text stops.
    pub fn speak(&self, chars: usize) {
        self.control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .speech
            .add(chars, Instant::now());
    }
    /// Show a transient emotion for `seconds` over her mood; `seconds <= 0` clears it. Names may be
    /// any of [`EMOTIONS`], a model expression name, or an emotion from the override file; the
    /// renderer reports names it cannot match in `info["warnings"]`.
    pub fn emote(&self, name: &str, seconds: f32) {
        self.control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .emote(name, seconds, Instant::now());
    }
    /// Queue a one-shot gesture from [`GESTURES`] or a model motion group name. Gestures play one
    /// after another; at most eight wait.
    pub fn act(&self, name: &str) {
        self.control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .act(name);
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
    /// Where the emotion/gesture overrides came from and what was dropped, or null.
    emotion_map: Value,
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
/// JSON files the Live2D runtime loads on demand: pose, user data, expressions and motions.
fn optional_references(definition: &Value) -> Vec<String> {
    let refs = &definition["FileReferences"];
    let mut found: Vec<&str> = ["Pose", "UserData"]
        .iter()
        .filter_map(|key| refs[*key].as_str())
        .collect();
    if let Some(list) = refs["Expressions"].as_array() {
        found.extend(list.iter().filter_map(|e| e["File"].as_str()));
    }
    if let Some(groups) = refs["Motions"].as_object() {
        for motions in groups.values().filter_map(Value::as_array) {
            found.extend(motions.iter().filter_map(|m| m["File"].as_str()));
        }
    }
    let mut out: Vec<String> = found
        .into_iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".json"))
        .map(String::from)
        .collect();
    out.sort();
    out.dedup();
    out
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
    let model = cfg.pet.join("弄玉运行档_无水印");
    let model_file = model.join("弄玉.model3.json");
    let definition: Value = serde_json::from_slice(
        &fs::read(&model_file)
            .context("Cannot find 弄玉.model3.json; set --pet-dir to desktop-pet/assets")?,
    )?;
    let mut paths = HashMap::<String, PathBuf>::new();
    paths.insert("model/弄玉.model3.json".into(), model_file);
    for key in ["Moc", "Physics", "DisplayInfo"] {
        if let Some(p) = definition["FileReferences"][key].as_str() {
            paths.insert(format!("model/{p}"), model.join(p));
        }
    }
    for p in definition["FileReferences"]["Textures"]
        .as_array()
        .context("No Live2D textures")?
    {
        let p = p.as_str().context("Invalid texture name")?;
        paths.insert(format!("model/{p}"), model.join(p));
    }
    for name in [
        "pixi.min.js",
        "live2dcubismcore.min.js",
        "pixi-live2d-display-cubism4.min.js",
    ] {
        paths.insert(format!("vendor/{name}"), cfg.pet.join("vendor").join(name));
    }
    let root = cfg.pet.canonicalize()?;
    for path in paths.values() {
        if !path.canonicalize()?.starts_with(&root) {
            bail!("Live2D reference escapes the asset directory")
        }
    }
    // Optional files follow the same rule; a missing one is left unserved rather than failing startup.
    let mut unserved = Vec::new();
    for reference in optional_references(&definition) {
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
    let (map, emotion_map) = load_emotion_map(&emotion_map_paths(&cfg.pet));
    let settings = json!({"texture_size":cfg.texture_size,"emotion_map":map})
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
        emotion_map,
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
    control: &Arc<Mutex<Control>>,
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
        if !server.emotion_map.is_null() {
            s.info["emotion_map"] = server.emotion_map.clone();
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
            m
        };
        let (view, speech, speech_source, emotion, act) = {
            let mut c = control.lock().unwrap_or_else(|e| e.into_inner());
            let (speech, source) = c.speech_for(&motion.state, frame_start);
            let emotion = c.transient(frame_start);
            (c.view, speech, source, emotion, c.acts.pop_front())
        };
        let input = json!({
            "state": motion.state,
            "mood": motion.mood,
            "tap": motion.tap,
            "look": motion.look,
            "speech": round1(speech),
            "view": [view.0, view.1],
            "format": format.page_name(),
            "emotion": emotion,
            "act": act,
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
            for key in ["emotion", "gesture", "actions", "last_action"] {
                s.info[key] = value[key].clone();
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
                s.info["speech_source"] = json!(speech_source);
                s.info["motion_playing"] = value["motion"].clone();
                s.info["expression"] = value["expression"].clone();
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
pub fn probe(cfg: &Config, dir: &Path) -> Result<()> {
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
                // Streamed text keeps her speech energy up, so the mouth moves as in a real reply.
                c.speak(9);
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
                c.speak(9);
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
            texture_size: 1024,
            limits: Default::default(),
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
        // Overrides come from the pet directory or ~/.config; this fixture has none of its own.
        assert!(settings.get("emotion_map").is_some());
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
        for name in [
            "model/sounds/idle.wav",
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

        // A reference that resolves outside the asset directory refuses to start the server.
        let outside = tempfile::tempdir().unwrap();
        let pet = pet_fixture(
            outside.path(),
            json!({"Expressions": [{"Name": "Leak", "File": "../../secret.exp3.json"}]}),
        );
        write(&outside.path().join("secret.exp3.json"), "{}");
        let cfg = config(outside.path(), pet, outside.path().join("chrome"));
        let error = assets(&cfg).err().expect("escaping reference must fail");
        assert!(error.to_string().contains("escapes"));
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
        let view = || companion.control.lock().unwrap().view;
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
    fn speech_energy_rises_with_text_and_decays_after_it_stops() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut meter = SpeechMeter::default();
        assert_eq!(meter.level(t0), 0.0);
        meter.add(7, t0);
        assert!((meter.level(t0) - 20.0).abs() < 1e-9);
        let decayed = meter.level(at(350));
        assert!((decayed - 20.0 * (-1f64).exp()).abs() < 1e-6, "{decayed}");
        assert_eq!(meter.level(at(2000)), 0.0);
        // A steady stream of 40 characters per second reads as about 40.
        let mut steady = SpeechMeter::default();
        for i in 0..60 {
            steady.add(2, at(50 * i));
        }
        let rate = steady.level(at(50 * 59));
        assert!((35.0..50.0).contains(&rate), "{rate}");
        // Silence returns her mouth to rest.
        assert_eq!(steady.level(at(50 * 59 + 1500)), 0.0);
        let mut burst = SpeechMeter::default();
        burst.add(10_000, t0);
        assert_eq!(burst.level(t0), SPEECH_CAP);
        // Until any text is reported, the speaking state stands in for speech.
        let mut control = Control::new();
        assert_eq!(control.speech_for("speaking", t0), (STATE_SPEECH, "state"));
        assert_eq!(control.speech_for("idle", t0), (0.0, "text"));
        control.speech.add(0, t0);
        assert_eq!(control.speech_for("speaking", t0).1, "state");
        control.speech.add(5, t0);
        assert_eq!(control.speech_for("speaking", at(3000)), (0.0, "text"));
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

    #[test]
    fn emotion_and_gesture_requests_expire_and_queue() {
        assert_eq!(EMOTIONS.len(), 18);
        assert_eq!(GESTURES.len(), 12);
        // The renderer knows exactly the same canonical names.
        let page = include_str!("../live2d/renderer.html");
        let list = |name: &str| -> Vec<String> {
            let start = page.find(&format!("const {name}=[")).unwrap() + name.len() + 8;
            let end = start + page[start..].find(']').unwrap();
            page[start..end]
                .split(',')
                .map(|s| s.trim_matches('\'').to_string())
                .collect()
        };
        assert_eq!(list("EMOTIONS"), EMOTIONS);
        assert_eq!(list("GESTURES"), GESTURES);

        let t0 = Instant::now();
        let mut control = Control::new();
        assert_eq!(control.transient(t0), "");
        control.emote(" shy ", 1.5, t0);
        assert_eq!(control.transient(t0 + Duration::from_millis(1400)), "shy");
        assert_eq!(control.transient(t0 + Duration::from_millis(1600)), "");
        assert!(control.emotion.is_none(), "expired in Rust");
        control.emote("F03", 10.0, t0);
        control.emote("F03", 0.0, t0);
        assert_eq!(control.transient(t0), "", "zero seconds clears");
        control.emote("angry", f32::NAN, t0);
        assert_eq!(control.transient(t0), "");
        control.emote(&"x".repeat(NAME_LIMIT + 1), 5.0, t0);
        assert_eq!(control.transient(t0), "", "overlong names are refused");
        control.emote("sad", 1e9, t0);
        assert!(control.emotion.as_ref().unwrap().1 <= t0 + Duration::from_secs(3600));
        for name in ["wave", "", "  ", "nod\u{7}", "bow"] {
            control.act(name);
        }
        assert_eq!(control.acts, ["wave", "bow"]);
        for i in 0..20 {
            control.act(&format!("g{i}"));
        }
        assert_eq!(control.acts.len(), ACT_QUEUE);
        assert_eq!(control.acts.front().unwrap(), "g12", "oldest dropped first");

        // The public API reaches the same queue on a companion whose renderer failed to start.
        let root = tempfile::tempdir().unwrap();
        let cfg = config(
            root.path(),
            root.path().join("pet"),
            root.path().join("missing-chrome"),
        );
        let companion = Companion::start(cfg, Graphics::Iterm);
        companion.emote("love", 2.0);
        companion.act("cheer");
        let mut c = companion.control.lock().unwrap();
        assert_eq!(c.transient(Instant::now()), "love");
        assert_eq!(c.acts.pop_front().as_deref(), Some("cheer"));
    }
    #[test]
    fn emotion_map_overrides_are_validated() {
        let raw = json!({
            "emotions": {
                "Shy": {"params": {"ParamCheek": 1, "ParamOld": null, "ParamBad": "x", "ParamList": [1, 2]},
                        "expression": "害羞", "motion": null, "colour": "pink"},
                "smug": {"params": {"ParamStarEye": 0.5}}
            },
            "gestures": {
                "wave": {"motion": "Wave", "params": {"ParamArmR": [0, 10], "ParamTooMany": [1, 2, 3, 4, 5, 6, 7, 8, 9]},
                         "expression": "Happy"},
                "heart": "not an object"
            },
            "bogus": true
        });
        let (map, warnings) = sanitize_emotion_map(&raw);
        assert_eq!(
            map,
            json!({
                "emotions": {
                    "shy": {"params": {"ParamCheek": 1, "ParamOld": null}, "expression": "害羞", "motion": null},
                    "smug": {"params": {"ParamStarEye": 0.5}}
                },
                "gestures": {"wave": {"motion": "Wave", "params": {"ParamArmR": [0, 10]}}}
            })
        );
        let joined = warnings.join("\n");
        for expected in [
            "unknown key `bogus`",
            "`emotions.shy.params.ParamBad` needs a number or null",
            "`emotions.shy.params.ParamList` needs a number or null",
            "unknown key `emotions.shy.colour`",
            "`gestures.wave.params.ParamTooMany` needs a number, a list",
            "unknown key `gestures.wave.expression`",
            "`gestures.heart` must be an object",
        ] {
            assert!(joined.contains(expected), "{expected} in {joined}");
        }
        assert_eq!(sanitize_emotion_map(&json!([1])).0, Value::Null);

        // The first file found wins; broken or oversized files are reported, not used.
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
        assert_eq!(
            load_emotion_map(&[a.clone(), b.clone()]),
            (Value::Null, Value::Null)
        );
        fs::write(&b, r#"{"emotions":{"shy":{"params":{"ParamCheek":0.8}}}}"#).unwrap();
        let (map, info) = load_emotion_map(&[a.clone(), b.clone()]);
        assert_eq!(map["emotions"]["shy"]["params"]["ParamCheek"], 0.8);
        assert_eq!(info["source"], b.display().to_string());
        assert_eq!(info["warnings"], json!([]));
        fs::write(&a, "{not json").unwrap();
        let (map, info) = load_emotion_map(&[a.clone(), b.clone()]);
        assert_eq!(map, Value::Null);
        assert!(info["error"].as_str().unwrap().starts_with("invalid JSON"));
        fs::write(
            &a,
            format!("{{\"emotions\":{{}},\"pad\":\"{}\"}}", "x".repeat(70_000)),
        )
        .unwrap();
        let (map, info) = load_emotion_map(&[a, b]);
        assert_eq!(map, Value::Null);
        assert_eq!(info["error"], "larger than 64 KB");
        assert!(emotion_map_paths(Path::new("/pet"))[0].ends_with("aster-nongyu.json"));
    }

    // ---- Headless end-to-end check with a stub rig -------------------------------------------
    //
    // Runs renderer.html in a real Chromium through the real pipeline (asset server, CDP, pacing,
    // frame decoding) with tests/fixtures/live2d-stub.js standing in for Pixi and the Live2D SDK.
    // The stub draws the rendered parameter values into each PNG frame, so motion can be measured.
    //   ASTER_TEST_CHROME=/path/to/chrome cargo test stub_renderer -- --ignored --nocapture

    /// Stub parameters in the order the fixture draws them: id, min, max.
    const STUB_PARAMS: [(&str, f64, f64); 27] = [
        ("ParamAngleX", -30.0, 30.0),
        ("ParamAngleY", -30.0, 30.0),
        ("ParamAngleZ", -30.0, 30.0),
        ("ParamBodyAngleX", -10.0, 10.0),
        ("ParamBodyAngleZ", -10.0, 10.0),
        ("ParamEyeBallX", -1.0, 1.0),
        ("ParamEyeBallY", -1.0, 1.0),
        ("ParamEyeLOpen", 0.0, 1.0),
        ("ParamEyeROpen", 0.0, 1.0),
        ("ParamBreath", 0.0, 1.0),
        ("ParamMouthOpenY", 0.0, 1.0),
        ("ParamEyeLSmile", 0.0, 1.0),
        ("ParamTeers2", 0.0, 1.0),
        ("ParamTeers3", 0.0, 1.0),
        ("ParamMouthForm", -1.0, 1.0),
        ("ParamBrowLY", -1.0, 1.0),
        ("ParamBrowAngry", 0.0, 1.0),
        ("ParamCheek", 0.0, 1.0),
        ("ParamTear", 0.0, 1.0),
        ("ParamHeartEye", 0.0, 1.0),
        ("ParamStarEye", 0.0, 1.0),
        ("ParamAngry", 0.0, 1.0),
        ("ParamSweat", 0.0, 1.0),
        ("ParamArmRA", 0.0, 10.0),
        ("ParamArmLWave", -10.0, 10.0),
        ("ParamHandChin", 0.0, 1.0),
        ("ParamHairFront", -1.0, 1.0),
    ];
    const ANGLES: [&str; 5] = [
        "ParamAngleX",
        "ParamAngleY",
        "ParamAngleZ",
        "ParamBodyAngleX",
        "ParamBodyAngleZ",
    ];
    const EMOTION_TOGGLES: [&str; 11] = [
        "ParamTeers2",
        "ParamTeers3",
        "ParamCheek",
        "ParamTear",
        "ParamHeartEye",
        "ParamStarEye",
        "ParamAngry",
        "ParamSweat",
        "ParamArmRA",
        "ParamArmLWave",
        "ParamHandChin",
    ];
    struct Sample {
        phase: &'static str,
        sequence: u64,
        at: Instant,
        values: HashMap<&'static str, f64>,
        /// motions started, automatic idle requests, expressions applied, stub frames
        counters: [u32; 4],
        size: (u32, u32),
        actions: u64,
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
                "DisplayInfo": "stub.cdi3.json",
                "Motions": {
                    "Idle": [{"File": "motions/idle.motion3.json"}],
                    "TapBody": [{"File": "motions/tap.motion3.json"}],
                    "Wave": [{"File": "motions/wave.motion3.json"}],
                    "手势_鞠躬": [{"File": "motions/bow.motion3.json"}],
                    "Happy": [{"File": "motions/happy.motion3.json"}]
                },
                "Expressions": [
                    {"Name": "Happy", "File": "expressions/happy.exp3.json"},
                    {"Name": "害羞", "File": "expressions/shy.exp3.json"}
                ]
            },
            "Groups": [
                {"Target": "Parameter", "Name": "EyeBlink", "Ids": ["ParamEyeLOpen", "ParamEyeROpen"]},
                {"Target": "Parameter", "Name": "LipSync", "Ids": ["ParamMouthOpenY"]}
            ]
        });
        write(&model.join("弄玉.model3.json"), &definition.to_string());
        // Display names as a Chinese rig would have them.
        let names = [
            ("ParamAngleX", "角度 X", "face"),
            ("ParamTeers2", "Teers2", "emotion"),
            ("ParamTeers3", "Teers3", "emotion"),
            ("ParamMouthForm", "嘴型", "face"),
            ("ParamBrowLY", "左眉", "face"),
            ("ParamBrowAngry", "生气眉", "face"),
            ("ParamCheek", "脸红", "emotion"),
            ("ParamTear", "流泪", "emotion"),
            ("ParamHeartEye", "爱心眼", "emotion"),
            ("ParamStarEye", "星星眼", "emotion"),
            ("ParamAngry", "生气", "emotion"),
            ("ParamSweat", "汗", "emotion"),
            ("ParamArmRA", "右手抬起", "arm"),
            ("ParamArmLWave", "左手挥动", "arm"),
            ("ParamHandChin", "托腮", "arm"),
            ("ParamHairFront", "前发", "hair"),
        ];
        let cdi = json!({
            "Version": 3,
            "Parameters": names.iter().map(|(id, name, group)| json!({"Id": id, "GroupId": group, "Name": name})).collect::<Vec<_>>(),
            "ParameterGroups": [
                {"Id": "face", "Name": "脸"}, {"Id": "emotion", "Name": "表情"},
                {"Id": "arm", "Name": "手臂"}, {"Id": "hair", "Name": "头发"}
            ]
        });
        write(&model.join("stub.cdi3.json"), &cdi.to_string());
        write(&model.join("stub.moc3"), "stub");
        write(&model.join("stub.physics3.json"), "{}");
        for (file, seconds, amplitude) in [
            ("idle", 1.5, 6),
            ("tap", 0.8, 4),
            ("wave", 1.5, 5),
            ("bow", 1.4, 4),
            ("happy", 1.0, 3),
        ] {
            write(
                &model.join(format!("motions/{file}.motion3.json")),
                &json!({"Version": 3, "Meta": {"Duration": seconds}, "StubAmplitude": amplitude})
                    .to_string(),
            );
        }
        write(
            &model.join("expressions/happy.exp3.json"),
            r#"{"Type":"Live2D Expression","Parameters":[{"Id":"ParamEyeLSmile","Value":0.3,"Blend":"Add"}]}"#,
        );
        write(
            &model.join("expressions/shy.exp3.json"),
            r#"{"Type":"Live2D Expression","Parameters":[{"Id":"ParamEyeLSmile","Value":0.2,"Blend":"Add"}]}"#,
        );
        // The user's overrides: a gentler proud, an arm heart, and two things that must be refused.
        write(
            &pet.join("aster-nongyu.json"),
            r#"{"emotions":{"proud":{"params":{"ParamStarEye":0.5,"ParamBrowLY":1}}},
                "gestures":{"heart":{"params":{"ParamArmRA":[3,7]}}},"bogus":true}"#,
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
    fn decode_stub(frame: &PortraitFrame, phase: &'static str, actions: u64) -> Sample {
        let rgb = frame.rgb().expect("stub frames decode");
        let block = |k: u32| {
            let p = rgb.get_pixel(k * 4 + 1, 1).0;
            (p, (u32::from(p[0]) << 8) | u32::from(p[1]))
        };
        let values = STUB_PARAMS
            .iter()
            .enumerate()
            .map(|(k, (id, min, max))| {
                let (pixel, n) = block(k as u32);
                assert_eq!(pixel[2], 90, "parameter strip at {id}");
                (*id, min + f64::from(n) / 65535.0 * (max - min))
            })
            .collect();
        let first = STUB_PARAMS.len() as u32;
        let counters = [first, first + 1, first + 2, first + 3].map(|k| {
            let (pixel, n) = block(k);
            assert_eq!(pixel[2], 91, "counter strip");
            n
        });
        Sample {
            phase,
            sequence: frame.sequence,
            at: Instant::now(),
            values,
            counters,
            size: (frame.width, frame.height),
            actions,
        }
    }
    fn record(
        companion: &Companion,
        phase: &'static str,
        seconds: f64,
        samples: &mut Vec<Sample>,
        mut each: impl FnMut(&Companion),
    ) {
        let started = Instant::now();
        let mut last = samples.last().map_or(0, |s| s.sequence);
        while started.elapsed().as_secs_f64() < seconds {
            each(companion);
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
                let actions = state.info["actions"].as_u64().unwrap_or(0);
                samples.push(decode_stub(&frame, phase, actions));
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
    fn peak(samples: &[&Sample], id: &str) -> f64 {
        samples
            .iter()
            .map(|s| s.values[id])
            .fold(f64::MIN, f64::max)
    }
    fn fps_of(samples: &[Sample], name: &str) -> f64 {
        let s = phase(samples, name);
        let (first, last) = (s[0], s[s.len() - 1]);
        (last.sequence - first.sequence) as f64 / last.at.duration_since(first.at).as_secs_f64()
    }
    fn ids(list: &Value) -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect()
    }
    fn r3(v: f64) -> f64 {
        (v * 1000.0).round() / 1000.0
    }
    #[test]
    #[ignore = "needs Chromium: set ASTER_TEST_CHROME or install /opt/pw-browsers"]
    fn stub_renderer_moves_smoothly_through_the_real_pipeline() {
        let chrome = test_chrome().expect("set ASTER_TEST_CHROME to a Chromium executable");
        let root = tempfile::tempdir().unwrap();
        let (pet, wrapper) = stub_pet(root.path(), &chrome);
        let cfg = Config {
            texture_size: 512,
            ..config(root.path(), pet, wrapper)
        };
        let companion = Companion::start(cfg.clone(), Graphics::Kitty);
        wait_for_frames(&companion, 2);
        let info = companion.current().info;
        assert_eq!(info["layer_hook"], "afterMotionUpdate", "{info:#}");
        assert_eq!(info["parameters_missing"], json!(["ParamEyeRSmile"]));
        assert_eq!(info["texture_sizes"][0]["rendered"], json!([512, 512]));
        assert_eq!(info["framing"]["face_at"], json!(0.44));

        // Discovery: display names, groups, toggles by keyword, arms, expressions and motions.
        let rig = &info["rig"];
        let cheek = rig["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "ParamCheek")
            .unwrap();
        assert_eq!(
            cheek,
            &json!({"id":"ParamCheek","name":"脸红","group":"表情","min":0,"max":1,"default":0})
        );
        let expect_toggles = [
            ("shy", vec!["ParamCheek"]),
            ("love", vec!["ParamHeartEye", "ParamTeers3", "ParamCheek"]),
            ("cry", vec!["ParamTear", "ParamTeers2"]),
            ("angry", vec!["ParamAngry", "ParamTeers2", "ParamTeers3"]),
            ("excited", vec!["ParamStarEye"]),
            ("worried", vec!["ParamSweat"]),
            ("embarrassed", vec!["ParamCheek", "ParamSweat"]),
        ];
        for (emotion, expected) in &expect_toggles {
            let found = ids(&rig["emotions"][emotion]["params"]);
            for id in expected {
                assert!(
                    found.contains(&id.to_string()),
                    "{emotion}: {id} in {found:?}"
                );
            }
        }
        assert_eq!(
            rig["emotions"]["proud"]["params"],
            json!([{"id":"ParamStarEye","value":0.5}])
        );
        assert_eq!(rig["emotions"]["shy"]["expression"], "害羞");
        assert_eq!(rig["emotions"]["happy"]["expression"], "Happy");
        assert_eq!(rig["emotions"]["happy"]["motion"], "Happy");
        assert_eq!(
            rig["arms"],
            json!([
                {"id":"ParamArmRA","side":"R","role":"raise"},
                {"id":"ParamArmLWave","side":"L","role":"wave"},
                {"id":"ParamHandChin","side":"","role":"chin"}
            ])
        );
        let kinds: serde_json::Map<String, Value> = GESTURES
            .iter()
            .map(|g| (g.to_string(), rig["gestures"][g]["kind"].clone()))
            .collect();
        assert_eq!(
            Value::Object(kinds.clone()),
            json!({"wave":"motion","nod":"procedural","shake":"procedural","tilt":"procedural",
                   "think":"params","cheer":"params","heart":"params","cover":"procedural",
                   "stretch":"params","look_around":"procedural","fidget":"procedural","bow":"motion"})
        );
        assert_eq!(rig["gestures"]["wave"]["motion"], "Wave");
        assert_eq!(rig["gestures"]["bow"]["motion"], "手势_鞠躬");
        assert_eq!(
            rig["gestures"]["heart"]["params"],
            json!([{"id":"ParamArmRA","values":[3,7]}])
        );
        for protected in ["ParamMouthForm", "ParamBrowLY", "ParamBrowAngry"] {
            assert!(
                rig["protected"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(protected))
            );
        }
        let text = rig.to_string();
        for never in [
            "ParamBrowLY\",\"value",
            "ParamBrowAngry\",\"value",
            "ParamMouthForm\",\"v",
            "ParamHairFront\",\"v",
        ] {
            assert!(!text.contains(never), "{never} must not be driven");
        }
        assert!(
            info["emotion_map"]["warnings"]
                .to_string()
                .contains("bogus")
        );
        assert!(
            info["warnings"]
                .to_string()
                .contains("ParamBrowLY is protected")
        );

        // 1. Idle: never still, and a fidget or idle motion within 35 s.
        let mut samples = Vec::new();
        companion.motion("idle", "neutral", false, false);
        record(&companion, "idle", 36.0, &mut samples, |_| {});
        let idle = phase(&samples, "idle");
        let start = idle[0].at;
        let first_action = idle
            .iter()
            .find(|s| s.actions > idle[0].actions)
            .map(|s| s.at.duration_since(start).as_secs_f64());
        assert!(
            first_action.is_some_and(|s| s <= 35.0),
            "no fidget: {first_action:?}"
        );
        let mut stillest = f64::MAX;
        for window in 0..7 {
            let (from, to) = (window as f64 * 5.0, window as f64 * 5.0 + 5.0);
            let part: Vec<&&Sample> = idle
                .iter()
                .filter(|s| (from..to).contains(&s.at.duration_since(start).as_secs_f64()))
                .collect();
            let moved = ANGLES
                .iter()
                .map(|id| {
                    let v: Vec<f64> = part.iter().map(|s| s.values[id]).collect();
                    v.iter().copied().fold(f64::MIN, f64::max)
                        - v.iter().copied().fold(f64::MAX, f64::min)
                })
                .fold(0.0, f64::max);
            stillest = stillest.min(moved);
        }
        assert!(
            stillest >= 1.0,
            "a 5 s window moved only {stillest} degrees"
        );

        // 2. Emotions: toggles ease in, then return to rest after the transient expires.
        let emotions: [(&str, &str, &str, &[&str]); 7] = [
            ("shy", "emo shy", "after shy", &["ParamCheek"]),
            (
                "love",
                "emo love",
                "after love",
                &["ParamHeartEye", "ParamTeers3"],
            ),
            ("cry", "emo cry", "after cry", &["ParamTear", "ParamTeers2"]),
            (
                "angry",
                "emo angry",
                "after angry",
                &["ParamAngry", "ParamTeers2"],
            ),
            ("excited", "emo excited", "after excited", &["ParamStarEye"]),
            ("worried", "emo worried", "after worried", &["ParamSweat"]),
            ("happy", "emo happy", "after happy", &[]),
        ];
        let before = samples.last().unwrap().counters;
        for (emotion, during, after, toggles) in emotions {
            companion.emote(emotion, 1.6);
            record(&companion, during, 1.7, &mut samples, |_| {});
            record(&companion, after, 1.3, &mut samples, |_| {});
            let (on, off) = (phase(&samples, during), phase(&samples, after));
            for id in toggles {
                assert!(
                    peak(&on, id) >= 0.9,
                    "{emotion}: {id} peaked at {}",
                    peak(&on, id)
                );
                let rest = off.last().unwrap().values[id];
                assert!(rest <= 0.03, "{emotion}: {id} stayed at {rest}");
            }
            // Emotion toggles outside this emotion's catalog stay at rest. Arm parameters are
            // left out: an idle fidget such as a stretch may move them meanwhile.
            let catalog = rig["emotions"][emotion]["params"].to_string();
            for id in EMOTION_TOGGLES
                .iter()
                .filter(|id| !id.contains("Arm") && !id.contains("Hand"))
            {
                if !catalog.contains(&format!("\"{id}\"")) {
                    let stray = peak(&on, id);
                    assert!(stray <= 0.02, "{emotion} moved {id} to {stray}");
                }
            }
        }
        let after_emotions = samples.last().unwrap().counters;
        assert!(
            after_emotions[2] >= before[2] + 2,
            "害羞 and Happy expressions"
        );
        assert!(after_emotions[0] > before[0], "the Happy motion played");
        assert_eq!(companion.current().info["emotion"], "neutral");

        // 3. Gestures: motion when the rig has one, parameters otherwise, head and body always.
        companion.motion("listening", "neutral", false, false);
        record(&companion, "settle", 4.0, &mut samples, |_| {});
        let gestures: [(&str, &str, f64); 7] = [
            ("think", "g think", 3.8),
            ("wave", "g wave", 2.4),
            ("cheer", "g cheer", 2.6),
            ("heart", "g heart", 3.2),
            ("nod", "g nod", 1.6),
            ("cover", "g cover", 2.9),
            ("bow", "g bow", 2.2),
        ];
        let mut gesture_phases = Vec::new();
        for (name, label, seconds) in gestures {
            let counters = samples.last().unwrap().counters;
            let level = samples.last().unwrap().values["ParamAngleY"];
            companion.act(name);
            record(&companion, label, seconds, &mut samples, |_| {});
            gesture_phases.push(label);
            let s = phase(&samples, label);
            let end = s.last().unwrap();
            match name {
                "think" => {
                    assert!(peak(&s, "ParamHandChin") >= 0.9);
                    assert!(end.values["ParamHandChin"] <= 0.02);
                }
                "wave" | "bow" => assert!(end.counters[0] > counters[0], "{name} motion"),
                "cheer" => {
                    assert!(peak(&s, "ParamArmRA") >= 8.0);
                    assert!(end.values["ParamArmRA"] <= 0.5);
                }
                "heart" => {
                    let mid: Vec<&Sample> = s[s.len() / 4..s.len() * 3 / 4].to_vec();
                    let low = mid
                        .iter()
                        .map(|s| s.values["ParamArmRA"])
                        .fold(f64::MAX, f64::min);
                    assert!(
                        peak(&mid, "ParamArmRA") >= 6.0 && low <= 4.0,
                        "{low}..{}",
                        peak(&mid, "ParamArmRA")
                    );
                    assert!(end.values["ParamArmRA"] <= 0.5);
                }
                "nod" => {
                    let lowest = s
                        .iter()
                        .map(|s| s.values["ParamAngleY"])
                        .fold(f64::MAX, f64::min);
                    assert!(lowest <= level - 2.5, "nod from {level} to {lowest}");
                }
                "cover" => {
                    assert!(peak(&s, "ParamCheek") >= 0.8, "cover blushes");
                    assert!(end.values["ParamCheek"] <= 0.05);
                }
                _ => unreachable!(),
            }
        }
        let gesture_step = ANGLES
            .iter()
            .map(|id| max_step(&samples, &gesture_phases, id))
            .fold(0.0, f64::max);
        assert!(gesture_step <= 4.0, "{gesture_step}");

        // 4. Speech: the mouth moves only with speech energy.
        let mut last_text = Instant::now() - Duration::from_secs(1);
        companion.motion("speaking", "neutral", false, false);
        record(&companion, "speaking", 2.5, &mut samples, |c| {
            if last_text.elapsed() >= Duration::from_millis(70) {
                last_text = Instant::now();
                c.speak(6);
            }
        });
        record(&companion, "quiet", 2.5, &mut samples, |_| {});
        let mouth = |name: &str| -> Vec<f64> {
            phase(&samples, name)
                .iter()
                .map(|s| s.values["ParamMouthOpenY"])
                .collect()
        };
        assert!(mouth("idle").iter().all(|v| *v == 0.0));
        let speaking = mouth("speaking");
        let mouth_peak = speaking.iter().copied().fold(0.0, f64::max);
        let reversals = speaking
            .windows(3)
            .filter(|w| (w[1] - w[0]) * (w[2] - w[1]) < 0.0)
            .count();
        assert!(
            mouth_peak > 0.3 && reversals >= 5,
            "{mouth_peak} {reversals}"
        );
        let quiet = mouth("quiet");
        assert!(
            quiet[quiet.len() * 3 / 4..].iter().all(|v| *v == 0.0),
            "{quiet:?}"
        );

        // 5. The canvas follows set_view.
        companion.set_view(300, 300);
        record(&companion, "square", 1.0, &mut samples, |_| {});
        assert_eq!(samples.last().unwrap().size, (300, 300));
        let state = companion.current();

        // Smooth throughout the calm and emotional phases; protected parameters never move.
        let mut calm: Vec<&str> = vec!["idle", "settle", "speaking", "quiet"];
        for (_, during, after, _) in emotions {
            calm.push(during);
            calm.push(after);
        }
        let bounds = [
            ("ParamAngleX", 2.5),
            ("ParamAngleY", 2.5),
            ("ParamAngleZ", 2.5),
            ("ParamBodyAngleX", 1.0),
            ("ParamBodyAngleZ", 1.0),
            ("ParamEyeBallX", 0.3),
            ("ParamEyeBallY", 0.3),
            ("ParamBreath", 0.15),
            ("ParamCheek", 0.45),
            ("ParamHeartEye", 0.45),
            ("ParamTear", 0.45),
        ];
        let mut steps = serde_json::Map::new();
        for (id, bound) in bounds {
            let step = max_step(&samples, &calm, id);
            steps.insert(id.into(), json!(r3(step)));
            assert!(step <= bound, "{id} jumped {step} in one frame");
        }
        for s in &samples {
            assert!(s.values["ParamMouthForm"].abs() < 1e-4);
            assert!(s.values["ParamBrowLY"].abs() < 1e-4);
            assert_eq!(s.values["ParamBrowAngry"], 0.0);
            assert!(s.values["ParamHairFront"].abs() < 1e-4);
        }
        let fps = state.info["fps"].as_f64().unwrap();
        assert!(fps >= 8.0, "{fps}");
        let mut summary = json!({
            "fps_reported": fps,
            "fps_idle": r3(fps_of(&samples, "idle")),
            "frame_ms": state.info["frame_ms"],
            "page_ms": state.info["page_ms"],
            "frames_sampled": samples.len(),
            "first_idle_action_s": first_action.map(r3),
            "stillest_5s_window_deg": r3(stillest),
            "max_step_calm_and_emotions": steps,
            "max_angle_step_in_gestures": r3(gesture_step),
            "gesture_kinds": kinds,
            "mouth_peak": r3(mouth_peak),
            "mouth_reversals": reversals,
            "actions": state.info["actions"],
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
