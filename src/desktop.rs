//! Desktop app mode: double-click `stdf-cypher.exe` (or drop STDF files on it)
//! and the dashboard opens in a chromeless app window. The app exits when the
//! window is closed. A second launch hands its files to the running instance.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const APP_NAME: &str = "STDF Cypher";

/// Per-user settings directory (recent files, instance lock).
/// `STDF_CYPHER_HOME` overrides both settings and cache locations.
pub fn config_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("STDF_CYPHER_HOME") {
        return PathBuf::from(h).join("config");
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("stdf-cypher")
}

/// Per-user cache directory (uploaded files, sample data, browser profile).
pub fn cache_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("STDF_CYPHER_HOME") {
        return PathBuf::from(h).join("cache");
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("stdf-cypher")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecentFile {
    pub path: String,
    pub opened: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub recent: Vec<RecentFile>,
}

impl Settings {
    fn file() -> PathBuf {
        config_dir().join("settings.json")
    }
    pub fn load() -> Settings {
        std::fs::read_to_string(Self::file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(config_dir());
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::file(), s);
        }
    }
    pub fn add_recent(&mut self, paths: &[String]) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let uploads = cache_dir().join("uploads");
        for p in paths {
            // uploaded copies are temporary – don't remember them
            if Path::new(p).starts_with(&uploads) {
                continue;
            }
            self.recent.retain(|r| &r.path != p);
            self.recent.insert(
                0,
                RecentFile {
                    path: p.clone(),
                    opened: now,
                },
            );
        }
        self.recent.truncate(15);
    }
}

/// Shared state for the desktop session (lives inside the API registry).
pub struct Session {
    pub app_mode: bool,
    pub started: Instant,
    /// open event-stream connections from app windows
    pub connections: Mutex<usize>,
    /// when the last window connection dropped (None while connected / never connected)
    pub disconnected_at: Mutex<Option<Instant>>,
    pub ever_connected: Mutex<bool>,
    pub pending_open: Mutex<Vec<String>>,
    /// pushes "open these files" to connected windows
    pub events: tokio::sync::broadcast::Sender<String>,
    pub settings: Mutex<Settings>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            app_mode: false,
            started: Instant::now(),
            connections: Mutex::new(0),
            disconnected_at: Mutex::new(None),
            ever_connected: Mutex::new(false),
            pending_open: Mutex::new(vec![]),
            events: tokio::sync::broadcast::channel(16).0,
            settings: Mutex::new(Settings::load()),
        }
    }
}

impl Session {
    /// Kept for the page's periodic poll (pending files); liveness uses the event stream.
    pub fn ping(&self) {}
    pub fn bye(&self) {}
    pub fn connect(&self) {
        *self.connections.lock().unwrap() += 1;
        *self.ever_connected.lock().unwrap() = true;
        *self.disconnected_at.lock().unwrap() = None;
    }
    pub fn disconnect(&self) {
        let mut c = self.connections.lock().unwrap();
        *c = c.saturating_sub(1);
        if *c == 0 {
            *self.disconnected_at.lock().unwrap() = Some(Instant::now());
        }
    }
    /// True when every app window has been closed (a reload reconnects within the grace period).
    pub fn should_exit(&self) -> bool {
        if !self.app_mode {
            return false;
        }
        if *self.connections.lock().unwrap() > 0 {
            return false;
        }
        if !*self.ever_connected.lock().unwrap() {
            // the window never connected (e.g. it failed to open)
            return self.started.elapsed() > Duration::from_secs(180);
        }
        self.disconnected_at
            .lock()
            .unwrap()
            .map(|t| t.elapsed() > Duration::from_secs(10))
            .unwrap_or(false)
    }
    pub fn queue_open(&self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        // deliver instantly to an open window; otherwise keep for the next poll
        let msg = serde_json::to_string(&paths).unwrap_or_default();
        if self.events.send(msg).is_err() {
            self.pending_open.lock().unwrap().extend(paths);
        }
    }
    pub fn remember(&self, paths: &[String]) {
        let mut s = self.settings.lock().unwrap();
        s.add_recent(paths);
        s.save();
    }
}

#[derive(Serialize, Deserialize)]
struct InstanceInfo {
    port: u16,
    pid: u32,
}

fn instance_file() -> PathBuf {
    config_dir().join("instance.json")
}

pub fn write_instance(port: u16) {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(
        instance_file(),
        serde_json::to_string(&InstanceInfo {
            port,
            pid: std::process::id(),
        })
        .unwrap_or_default(),
    );
}

pub fn clear_instance() {
    if let Ok(s) = std::fs::read_to_string(instance_file()) {
        if let Ok(i) = serde_json::from_str::<InstanceInfo>(&s) {
            if i.pid == std::process::id() {
                let _ = std::fs::remove_file(instance_file());
            }
        }
    }
}

/// Minimal HTTP POST to a running instance (no HTTP client dependency).
fn post_local(port: u16, path: &str, body: &str) -> Result<String> {
    let mut s =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(800))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

/// If another instance is running, give it our files and return true.
pub fn hand_off_to_running(files: &[String]) -> bool {
    let Ok(s) = std::fs::read_to_string(instance_file()) else {
        return false;
    };
    let Ok(info) = serde_json::from_str::<InstanceInfo>(&s) else {
        return false;
    };
    let body = serde_json::json!({ "paths": files, "focus": true }).to_string();
    match post_local(info.port, "/api/open", &body) {
        Ok(resp) => resp.contains("\"ok\":true") && resp.contains("stdf-cypher"),
        Err(_) => false,
    }
}

/// Find a Chromium-based browser that supports `--app` mode.
pub fn find_app_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STDF_CYPHER_BROWSER") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let mut cands: Vec<PathBuf> = vec![];
    if cfg!(windows) {
        for var in [
            "ProgramFiles(x86)",
            "ProgramFiles",
            "LOCALAPPDATA",
            "ProgramW6432",
        ] {
            if let Some(base) = std::env::var_os(var) {
                let b = PathBuf::from(base);
                cands.push(b.join(r"Microsoft\Edge\Application\msedge.exe"));
                cands.push(b.join(r"Google\Chrome\Application\chrome.exe"));
                cands.push(b.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
                cands.push(b.join(r"Chromium\Application\chrome.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        cands.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        cands.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
        cands.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
        cands.push("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".into());
    } else if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for n in [
                "microsoft-edge",
                "microsoft-edge-stable",
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
                "brave-browser",
            ] {
                cands.push(dir.join(n));
            }
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

/// Open the dashboard in an app window (no address bar). Falls back to the default browser.
pub fn open_window(url: &str) -> Result<()> {
    if let Some(b) = find_app_browser() {
        let profile = cache_dir().join("window-profile");
        let _ = std::fs::create_dir_all(&profile);
        let res = std::process::Command::new(&b)
            .arg(format!("--app={url}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--window-size=1440,920")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-features=Translate,msEdgeSidebarV2")
            .spawn();
        if res.is_ok() {
            return Ok(());
        }
    }
    open_default_browser(url)
}

pub fn open_default_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    r.map(|_| ()).context("could not open a browser")
}

/// Show a folder in the file manager (selecting the file when given one).
pub fn reveal(path: &str) -> Result<()> {
    let p = Path::new(path);
    #[cfg(target_os = "windows")]
    {
        if p.is_file() {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", p.display()))
                .spawn()?;
        } else {
            std::process::Command::new("explorer").arg(p).spawn()?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(p)
            .spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if p.is_file() {
            p.parent().unwrap_or(p)
        } else {
            p
        };
        std::process::Command::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}

// ------------------------------------------------------------- native dialogs

pub fn dialogs_available() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// On Windows, parent the dialogs to the app window so they open on top of it.
#[cfg(windows)]
mod owner {
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
        Win32WindowHandle, WindowHandle,
    };
    use std::num::NonZeroIsize;
    pub struct Owner(NonZeroIsize);
    impl HasWindowHandle for Owner {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.0));
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }
    impl HasDisplayHandle for Owner {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }
    /// The app window (Edge/Chrome --app window titled like the page).
    pub fn find() -> Option<Owner> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow};
        let title: Vec<u16> = super::APP_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        let h = NonZeroIsize::new(hwnd as isize)?;
        unsafe { SetForegroundWindow(hwnd) };
        Some(Owner(h))
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod dialogs {
    use std::path::PathBuf;
    fn base() -> rfd::FileDialog {
        let d = rfd::FileDialog::new();
        #[cfg(windows)]
        let d = match super::owner::find() {
            Some(o) => d.set_parent(&o),
            None => d,
        };
        d
    }
    pub fn pick_files(dir: Option<PathBuf>) -> Vec<PathBuf> {
        let mut d = base()
            .set_title("Open STDF files")
            .add_filter("STDF files", &["stdf", "std", "gz", "stdf_gz", "stdfz"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        d.pick_files().unwrap_or_default()
    }
    pub fn pick_folder(title: &str) -> Option<PathBuf> {
        base().set_title(title).pick_folder()
    }
    pub fn pick_save(title: &str, name: &str, ext: &str) -> Option<PathBuf> {
        let mut d = base().set_title(title).set_file_name(name);
        if !ext.is_empty() {
            d = d.add_filter(ext.to_uppercase(), &[ext]);
        }
        d.save_file()
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod dialogs {
    use std::path::PathBuf;
    pub fn pick_files(_dir: Option<PathBuf>) -> Vec<PathBuf> {
        vec![]
    }
    pub fn pick_folder(_title: &str) -> Option<PathBuf> {
        None
    }
    pub fn pick_save(_title: &str, _name: &str, _ext: &str) -> Option<PathBuf> {
        None
    }
}

pub use dialogs::{pick_files, pick_folder, pick_save};

// ------------------------------------------------------------- Windows console

/// When the exe was double-clicked, Windows gives it a fresh console window.
/// Hide it in app mode (but keep it when started from an existing terminal).
#[cfg(windows)]
pub fn detach_console_if_owned() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    unsafe {
        let mut pids = [0u32; 4];
        let n = GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32);
        if n == 1 {
            FreeConsole();
        }
    }
}

#[cfg(not(windows))]
pub fn detach_console_if_owned() {}

/// Show an error to a user who may have no console.
pub fn fatal(msg: &str) {
    eprintln!("error: {msg}");
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let w = |s: &str| {
            s.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let (t, m) = (w(APP_NAME), w(msg));
        MessageBoxW(
            std::ptr::null_mut(),
            m.as_ptr(),
            t.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Create the three demo files (wafer sort lot A, shifted lot B, lot A on a second tester).
pub fn make_samples() -> Result<Vec<String>> {
    use crate::sample::{generate, SampleSpec};
    let dir = cache_dir().join("samples");
    std::fs::create_dir_all(&dir)?;
    let specs = [
        (
            "DEMO_lotA.stdf",
            SampleSpec {
                lot_id: "DEMO_A".into(),
                ..Default::default()
            },
        ),
        (
            "DEMO_lotB_shifted.stdf",
            SampleSpec {
                lot_id: "DEMO_B".into(),
                seed: 2,
                shift: 0.6,
                ..Default::default()
            },
        ),
        (
            "DEMO_lotA_tester2.stdf",
            SampleSpec {
                lot_id: "DEMO_A".into(),
                meas_noise: 0.3,
                tester_offset: 1.5,
                ..Default::default()
            },
        ),
    ];
    let mut out = vec![];
    for (name, spec) in specs {
        let p = dir.join(name);
        if !p.exists() {
            generate(&p, &spec)?;
        }
        out.push(p.to_string_lossy().to_string());
    }
    Ok(out)
}

/// Remove stale uploaded copies from previous sessions.
pub fn clean_uploads() {
    let dir = cache_dir().join("uploads");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| {
                    t.elapsed()
                        .map(|d| d > Duration::from_secs(86400))
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if old {
                let p = e.path();
                let _ = if p.is_dir() {
                    std::fs::remove_dir_all(p)
                } else {
                    std::fs::remove_file(p)
                };
            }
        }
    }
}
