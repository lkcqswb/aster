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
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

#[derive(Clone, Debug, Default)]
pub struct Motion {
    pub state: String,
    pub mood: String,
    pub tap: bool,
    pub look: bool,
}
#[derive(Clone)]
pub struct PortraitFrame {
    pub png: Vec<u8>,
    pub sequence: u64,
    pub image: Arc<image::DynamicImage>,
}
#[derive(Clone, Default)]
pub struct Shared {
    pub frame: Option<PortraitFrame>,
    pub status: String,
    pub frames: u64,
    pub motion: Motion,
    pub info: Value,
}
pub struct Companion {
    pub shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Companion {
    pub fn start(cfg: Config) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: "Opening her room…".into(),
            motion: Motion {
                state: "idle".into(),
                mood: "neutral".into(),
                ..Default::default()
            },
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let sh = shared.clone();
        let st = stop.clone();
        let handle = thread::spawn(move || {
            for attempt in 1..=2 {
                sh.lock().unwrap_or_else(|e| e.into_inner()).info["attempt"] = json!(attempt);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    render_loop(&cfg, &sh, &st)
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
    let settings = json!({"texture_size":cfg.texture_size})
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
                    paths.get(name).and_then(|p| fs::read(p).ok()).map(|b| {
                        (
                            b,
                            if name.ends_with(".js") {
                                "application/javascript"
                            } else if name.ends_with(".png") {
                                "image/png"
                            } else {
                                "application/octet-stream"
                            },
                        )
                    })
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
fn render_loop(cfg: &Config, shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>) -> Result<()> {
    report(cfg, shared, "Finding her model…", None);
    if !cfg.chrome.is_file() {
        bail!("Chrome/Chromium not found; set ASTER_CHROME")
    }
    let server = assets(cfg)?;
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
    let mut previous = Instant::now();
    while !stop.load(Ordering::Relaxed) && !crate::lifecycle::requested() {
        let frame_start = Instant::now();
        let dt = previous.elapsed().as_millis().clamp(16, 250);
        previous = Instant::now();
        let motion = {
            let mut s = shared.lock().unwrap();
            let m = s.motion.clone();
            s.motion.tap = false;
            s.motion.look = false;
            m
        };
        let state =
            json!({"state":motion.state,"mood":motion.mood,"tap":motion.tap,"look":motion.look});
        let value = cdp.evaluate(&format!("window.asterFrame({state},{dt})"))?;
        let png = STANDARD.decode(
            value["png"]
                .as_str()
                .context("Renderer returned no frame")?,
        )?;
        let image = Arc::new(image::load_from_memory(&png)?);
        {
            let mut s = shared.lock().unwrap();
            s.frames += 1;
            s.frame = Some(PortraitFrame {
                png,
                sequence: s.frames,
                image,
            });
        }
        if shared.lock().unwrap().frames == 1 {
            report(cfg, shared, "Live2D · connected", None);
        }
        // The native model updates every captured frame. 8 fps keeps terminal bandwidth and CPU bounded.
        let rest = Duration::from_millis(125).saturating_sub(frame_start.elapsed());
        if !rest.is_zero() {
            thread::sleep(rest);
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
    pub fn encode(self, png: &[u8], area: Rect) -> String {
        let data = STANDARD.encode(png);
        let mut s = format!("\x1b7\x1b[{};{}H", area.y + 1, area.x + 1);
        match self {
            Self::Iterm => {
                s += &format!(
                    "\x1b]1337;File=inline=1;width={};height={};preserveAspectRatio=1:{}\x07",
                    area.width, area.height, data
                )
            }
            Self::Kitty => {
                for (i, chunk) in data.as_bytes().chunks(4096).enumerate() {
                    let more = usize::from((i + 1) * 4096 < data.len());
                    let params = if i == 0 {
                        format!(
                            "a=T,f=100,i=731,p=1,q=2,C=1,c={},r={},m={more}",
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
            }
            _ => {}
        }
        s + "\x1b8"
    }
    pub fn clear(self) -> &'static str {
        if self == Self::Kitty {
            "\x1b_Ga=d,d=I,i=731,q=2\x1b\\"
        } else {
            ""
        }
    }
}
pub fn halfblocks(frame: &PortraitFrame, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    };
    let im = frame
        .image
        .resize_exact(
            area.width.into(),
            u32::from(area.height) * 2,
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8();
    for y in 0..area.height {
        for x in 0..area.width {
            let a = im.get_pixel(x.into(), u32::from(y) * 2).0;
            let b = im.get_pixel(x.into(), u32::from(y) * 2 + 1).0;
            buf[(area.x + x, area.y + y)]
                .set_symbol("▀")
                .set_fg(Color::Rgb(a[0], a[1], a[2]))
                .set_bg(Color::Rgb(b[0], b[1], b[2]));
        }
    }
}
pub fn probe(cfg: &Config, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let c = Companion::start(cfg.clone());
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
                if frame.sequence < *sequence + 8 {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                fs::write(dir.join("speaking.png"), &frame.png)?;
                if &frame.png == initial {
                    bail!("Animation frames were identical")
                };
                fs::write(
                    dir.join("renderer.json"),
                    serde_json::to_vec_pretty(
                        &json!({"info":s.info,"frames":s.frames,"animation_changes":true}),
                    )?,
                )?;
                println!(
                    "Live2D model loaded; {} distinct frame intervals captured in {}",
                    s.frames,
                    dir.display()
                );
                return Ok(());
            } else {
                fs::write(dir.join("idle.png"), &frame.png)?;
                first = Some((frame.png, frame.sequence));
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
    #[test]
    fn renderer_settings_are_local_and_asset_routes_stay_bounded() {
        let root = tempfile::tempdir().unwrap();
        let pet = root.path().join("assets");
        let model = pet.join("弄玉运行档_无水印");
        fs::create_dir_all(&model).unwrap();
        fs::create_dir_all(pet.join("vendor")).unwrap();
        fs::write(
            model.join("弄玉.model3.json"),
            r#"{"FileReferences":{"Moc":"model.moc3","Textures":["texture.png"]}}"#,
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
        let cfg = Config {
            home: root.path().into(),
            project: root.path().into(),
            state: root.path().join("state"),
            key: String::new(),
            base: String::new(),
            model: String::new(),
            pet,
            chrome: root.path().join("chrome"),
            texture_size: 1024,
        };
        let server = assets(&cfg).unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let settings: Value = client
            .get(format!("{}settings.json", server.url))
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(settings, json!({"texture_size":1024}));
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
    fn graphics_packets_are_inline_and_chunked() {
        let a = Rect::new(2, 3, 20, 30);
        let it = Graphics::Iterm.encode(b"png", a);
        assert!(it.contains("inline=1;width=20;height=30"));
        let kitty = Graphics::Kitty.encode(&vec![0; 9000], a);
        assert!(kitty.contains("a=T,f=100"));
        assert!(kitty.contains("m=0"));
        assert!(kitty.ends_with("\x1b8"));
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
            home: root.path().into(),
            project: root.path().into(),
            state: root.path().join("state"),
            key: "must-not-appear-in-log".into(),
            base: String::new(),
            model: String::new(),
            pet: root.path().join("pet"),
            chrome: root.path().join("missing-chrome"),
            texture_size: 2048,
        };
        let companion = Companion::start(cfg.clone());
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
}
