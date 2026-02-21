use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub const SERVER_STARTING: u8 = 0;
pub const SERVER_READY: u8 = 1;
pub const SERVER_FAILED: u8 = 2;

#[cfg(target_os = "windows")]
const VENV_SCRIPT_DIR: &str = "Scripts";
#[cfg(target_os = "windows")]
const LT_EXECUTABLE: &str = "libretranslate.exe";

#[cfg(target_os = "macos")]
const VENV_SCRIPT_DIR: &str = "bin";
#[cfg(target_os = "macos")]
const LT_EXECUTABLE: &str = "libretranslate";

pub struct LibreTranslateServer {
    child: Arc<Mutex<Child>>,
    port: u16,
}

impl LibreTranslateServer {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Shared handle for the readiness monitor to call try_wait() on.
    pub fn child_handle(&self) -> Arc<Mutex<Child>> {
        Arc::clone(&self.child)
    }

    pub fn start_background(
        python_path: Option<&str>,
        preferred_port: u16,
        load_languages: &str,
    ) -> anyhow::Result<Self> {
        Self::start_impl(python_path, preferred_port, load_languages, false)
    }

    fn start_impl(
        python_path: Option<&str>,
        preferred_port: u16,
        load_languages: &str,
        wait_ready: bool,
    ) -> anyhow::Result<Self> {
        let exe = Self::find_executable(python_path)?;

        // Find available port (try preferred, then next 10 ports)
        let port = (preferred_port..preferred_port + 10)
            .find(|&p| is_port_available(p))
            .ok_or_else(|| anyhow::anyhow!(
                "No available ports in range {}-{}",
                preferred_port,
                preferred_port + 9
            ))?;

        if port != preferred_port {
            tracing::info!(
                "Port {} occupied, using port {} instead",
                preferred_port,
                port
            );
        }

        tracing::info!("Starting LibreTranslate: {} (port {})", exe.display(), port);

        let mut cmd = Command::new(&exe);

        let is_python = exe
            .file_name()
            .and_then(|f| f.to_str())
            .map(|f| f.eq_ignore_ascii_case("python.exe") || f.eq_ignore_ascii_case("python") || f.eq_ignore_ascii_case("python3"))
            .unwrap_or(false);
        if is_python {
            if let Some(parent) = exe.parent() {
                #[cfg(target_os = "windows")]
                {
                    // Check for embedded Python layout (python.exe at libretranslate/python.exe)
                    // parent = libretranslate/
                    let embedded_main = parent
                        .join("Lib")
                        .join("site-packages")
                        .join("libretranslate")
                        .join("main.py");

                    if embedded_main.exists() {
                        tracing::debug!("Using embedded Python with main.py: {}", embedded_main.display());
                        cmd.arg(embedded_main);
                    } else if let Some(venv_root) = parent.parent() {
                        // Check for venv layout (python.exe at libretranslate/Scripts/python.exe)
                        // parent = Scripts/, venv_root = libretranslate/
                        let venv_main = venv_root
                            .join("Lib")
                            .join("site-packages")
                            .join("libretranslate")
                            .join("main.py");

                        if venv_main.exists() {
                            tracing::debug!("Using venv with main.py: {}", venv_main.display());
                            cmd.arg(venv_main);
                        } else {
                            tracing::debug!("main.py not found, using -m libretranslate");
                            cmd.args(["-m", "libretranslate"]);
                        }
                    } else {
                        tracing::debug!("Using -m libretranslate");
                        cmd.args(["-m", "libretranslate"]);
                    }
                }

                #[cfg(target_os = "macos")]
                if let Some(venv_root) = parent.parent() {
                    // Check lib/python3.*/site-packages/
                    let mut main_py = None;
                    if let Ok(lib_dir) = std::fs::read_dir(venv_root.join("lib")) {
                        for entry in lib_dir.flatten() {
                            if entry.file_name().to_string_lossy().starts_with("python3") {
                                let candidate = entry.path()
                                    .join("site-packages")
                                    .join("libretranslate")
                                    .join("main.py");
                                if candidate.exists() {
                                    main_py = Some(candidate);
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(script_path) = main_py {
                        tracing::debug!("Using macOS venv main.py: {}", script_path.display());
                        cmd.arg(script_path);
                    } else {
                        tracing::debug!("Using -m libretranslate");
                        cmd.args(["-m", "libretranslate"]);
                    }
                } else {
                    tracing::debug!("Using -m libretranslate");
                    cmd.args(["-m", "libretranslate"]);
                }
            } else {
                cmd.args(["-m", "libretranslate"]);
            }
        }

        cmd.args(["--host", "127.0.0.1", "--port", &port.to_string()]);

        // Set ARGOS_PACKAGES_DIR to bundled packages location if available
        if let Some(bundled_dir) = Self::find_bundled_packages(&exe) {
            tracing::info!("Using bundled language packages: {}", bundled_dir.display());
            cmd.env("ARGOS_PACKAGES_DIR", &bundled_dir);
        }

        // Only use --load-only if language packages are already installed.
        // On first launch, use --update-models so LibreTranslate downloads them.
        if Self::has_language_packages(&exe) {
            // Prefer installer manifest over config (it reflects what was actually installed)
            let effective_languages = Self::read_installed_languages(&exe)
                .unwrap_or_else(|| load_languages.to_string());
            cmd.args(["--load-only", &effective_languages]);
        } else {
            tracing::info!("No language packages found — first launch will download models");
            cmd.arg("--update-models");
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null());

        // Capture stderr to a log file for diagnostics when LibreTranslate fails
        let stderr_path = crate::config::Config::app_dir().join("libretranslate.log");
        match std::fs::File::create(&stderr_path) {
            Ok(f) => {
                tracing::info!("LibreTranslate stderr → {}", stderr_path.display());
                cmd.stderr(Stdio::from(f));
            }
            Err(_) => {
                cmd.stderr(Stdio::null());
            }
        }

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }

        let child = cmd.spawn()?;

        tracing::info!("LibreTranslate started (PID: {})", child.id());

        let server = Self {
            child: Arc::new(Mutex::new(child)),
            port,
        };

        if wait_ready {
            server.wait_for_ready()?;
        } else {
            tracing::info!("Started in background - will be ready in ~5-10 seconds");
        }

        Ok(server)
    }

    /// Find bundled argos-translate packages next to the Python executable.
    /// Looks for an `argos-packages` directory alongside the Python env.
    fn find_bundled_packages(exe: &std::path::Path) -> Option<PathBuf> {
        // For embedded Python: exe = .../libretranslate/python.exe
        // bundled = .../libretranslate/argos-packages/
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("argos-packages");
            if dir_has_entries(&candidate) {
                return Some(candidate);
            }
            // For venv Python: exe = .../libretranslate/Scripts/python.exe
            if let Some(venv_root) = parent.parent() {
                let candidate = venv_root.join("argos-packages");
                if dir_has_entries(&candidate) {
                    return Some(candidate);
                }
            }
        }
        // Check next to the application executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let candidate = exe_dir.join("libretranslate").join("argos-packages");
                if dir_has_entries(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Check if argos-translate has any installed language packages.
    /// Checks both bundled packages and user-profile packages.
    fn has_language_packages(exe: &std::path::Path) -> bool {
        // Check bundled packages first
        if Self::find_bundled_packages(exe).is_some() {
            return true;
        }

        // Check user-profile packages
        #[cfg(target_os = "windows")]
        let home = std::env::var_os("USERPROFILE");
        #[cfg(target_os = "macos")]
        let home = std::env::var_os("HOME");

        if let Some(home) = home {
            let packages_dir = PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("argos-translate")
                .join("packages");
            if dir_has_entries(&packages_dir) {
                return true;
            }
        }
        false
    }

    /// Read the installer-generated `installed-languages.txt` manifest.
    /// Returns the contents (e.g. "en,zh,ja") if the file exists next to the Python env.
    fn read_installed_languages(exe: &std::path::Path) -> Option<String> {
        // Check next to Python exe (embedded layout: libretranslate/installed-languages.txt)
        let candidates = [
            exe.parent().map(|p| p.join("installed-languages.txt")),
            // Venv layout: libretranslate/Scripts/python.exe -> libretranslate/
            exe.parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("installed-languages.txt")),
            // Next to the application executable
            std::env::current_exe().ok().and_then(|e| {
                e.parent()
                    .map(|d| d.join("libretranslate").join("installed-languages.txt"))
            }),
        ];

        for candidate in candidates.iter().flatten() {
            if let Ok(contents) = std::fs::read_to_string(candidate) {
                let trimmed = contents.trim().to_string();
                if !trimmed.is_empty() {
                    tracing::info!(
                        "Using installer language manifest: {} (from {})",
                        trimmed,
                        candidate.display()
                    );
                    return Some(trimmed);
                }
            }
        }
        None
    }

    fn find_executable(python_path: Option<&str>) -> anyhow::Result<PathBuf> {
        let lt_script = [VENV_SCRIPT_DIR, LT_EXECUTABLE];

        // explicit config path
        if let Some(path) = python_path {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
            tracing::warn!("Configured python_path not found: {}", path);
        }

        // bundled Python next to exe (installer layout)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                #[cfg(target_os = "windows")]
                {
                    // Embedded Python (installer) - DON'T use Scripts/libretranslate.exe
                    // because it has hardcoded paths from build server
                    let embedded_py = exe_dir.join("libretranslate").join("python.exe");
                    tracing::debug!("Checking embedded Python: {}", embedded_py.display());
                    if embedded_py.exists() {
                        // Verify libretranslate is installed
                        let lt_module = exe_dir
                            .join("libretranslate")
                            .join("Lib")
                            .join("site-packages")
                            .join("libretranslate");
                        if lt_module.exists() {
                            tracing::debug!("Found embedded Python with libretranslate module");
                            return Ok(embedded_py);
                        }
                    }

                    // Venv Python (manual setup)
                    let venv_py = exe_dir
                        .join("libretranslate")
                        .join("Scripts")
                        .join("python.exe");
                    tracing::debug!("Checking venv Python: {}", venv_py.display());
                    if venv_py.exists() {
                        return Ok(venv_py);
                    }
                }

                // macOS .app bundle: Contents/MacOS/screen-translate -> Contents/Resources/
                #[cfg(target_os = "macos")]
                if let Some(contents_dir) = exe_dir.parent() {
                    let candidate = contents_dir
                        .join("Resources")
                        .join("libretranslate")
                        .join("bin")
                        .join("python3");
                    tracing::debug!("Checking .app bundle Python: {}", candidate.display());
                    if candidate.exists() {
                        return Ok(candidate);
                    }
                }
            }
        }

        // walk up from exe dir looking for libretranslate
        if let Ok(exe_path) = std::env::current_exe() {
            let mut dir = exe_path.as_path();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    let candidate: PathBuf =
                        [parent.to_str().unwrap_or("."), "libretranslate"]
                            .iter()
                            .chain(lt_script.iter())
                            .collect();
                    tracing::debug!("Checking venv: {}", candidate.display());
                    if candidate.exists() {
                        return Ok(candidate);
                    }
                    dir = parent;
                }
            }
        }

        // same walk from cwd
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = cwd.as_path();
            for _ in 0..3 {
                let candidate: PathBuf = dir
                    .join("libretranslate")
                    .join(VENV_SCRIPT_DIR)
                    .join(LT_EXECUTABLE);
                tracing::debug!("Checking cwd venv: {}", candidate.display());
                if candidate.exists() {
                    return Ok(candidate);
                }
                match dir.parent() {
                    Some(p) => dir = p,
                    None => break,
                }
            }
        }

        // last resort: PATH
        if let Ok(path) = which::which("libretranslate") {
            return Ok(path);
        }

        #[cfg(target_os = "windows")]
        anyhow::bail!(
            "LibreTranslate not found. Install it:\n\
             py -3.12 -m venv ../libretranslate\n\
             ../libretranslate/Scripts/pip install libretranslate"
        );
        #[cfg(target_os = "macos")]
        anyhow::bail!(
            "LibreTranslate not found. Install it:\n\
             python3 -m venv ../libretranslate\n\
             ../libretranslate/bin/pip install libretranslate"
        )
    }

    fn wait_for_ready(&self) -> anyhow::Result<()> {
        tracing::info!("Waiting for LibreTranslate to be ready...");

        for i in 0..45 {
            std::thread::sleep(Duration::from_secs(1));

            if std::net::TcpStream::connect(format!("127.0.0.1:{}", self.port)).is_ok() {
                tracing::info!("LibreTranslate ready after {}s", i + 1);
                return Ok(());
            }

            if (i + 1) % 10 == 0 {
                tracing::info!("Still waiting... ({}s)", i + 1);
            }
        }

        anyhow::bail!("LibreTranslate failed to start within 45s")
    }
}

impl Drop for LibreTranslateServer {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap();
        tracing::info!("Stopping LibreTranslate (PID: {})", child.id());
        // Only kill if still running (try_wait returns Ok(None) when alive)
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::info!("LibreTranslate already exited with {}", status);
            }
            _ => {
                let _ = child.kill();
            }
        }
        let _ = child.wait();
    }
}

fn dir_has_entries(path: &std::path::Path) -> bool {
    if let Ok(mut entries) = std::fs::read_dir(path) {
        return entries.next().is_some();
    }
    false
}

pub fn is_libretranslate_running(port: u16) -> bool {
    // Don't just check if port is open - verify it's actually LibreTranslate
    let url = format!("http://127.0.0.1:{}/languages", port);
    match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
    {
        Ok(client) => match client.get(&url).send() {
            Ok(resp) => {
                // Check if response looks like LibreTranslate /languages endpoint
                resp.status().is_success() && resp.text().unwrap_or_default().contains("code")
            }
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn is_port_available(port: u16) -> bool {
    std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_err()
}

/// Spawn a background thread that monitors whether LibreTranslate becomes ready.
/// Updates the shared `status` from SERVER_STARTING → SERVER_READY or SERVER_FAILED.
///
/// Uses `Child::try_wait()` for reliable crash detection on all platforms.
/// The previous macOS implementation used `kill(pid, 0)` which returns true
/// for zombie processes, causing the monitor to wait 180s before detecting
/// a crash instead of reporting it immediately.
pub fn spawn_readiness_monitor(
    port: u16,
    child: Arc<Mutex<Child>>,
    status: Arc<AtomicU8>,
) {
    std::thread::spawn(move || {
        let pid = child.lock().unwrap().id();
        let max_wait = Duration::from_secs(180); // first run downloads models
        let start = std::time::Instant::now();
        let mut last_log = 0u64;

        loop {
            std::thread::sleep(Duration::from_secs(2));

            // Check if the process crashed using try_wait (works correctly
            // on all platforms, including detecting zombies on macOS/Unix)
            {
                let mut child = child.lock().unwrap();
                match child.try_wait() {
                    Ok(Some(exit_status)) => {
                        tracing::error!(
                            "LibreTranslate process (PID {}) exited with: {}. \
                             Check libretranslate.log in app data folder for details.",
                            pid,
                            exit_status
                        );
                        status.store(SERVER_FAILED, Ordering::Relaxed);
                        return;
                    }
                    Ok(None) => {
                        // Still running, continue
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to check LibreTranslate process (PID {}): {}",
                            pid,
                            e
                        );
                        status.store(SERVER_FAILED, Ordering::Relaxed);
                        return;
                    }
                }
            }

            // Check if the HTTP endpoint is responding
            if is_libretranslate_running(port) {
                tracing::info!("LibreTranslate is ready on port {}", port);
                status.store(SERVER_READY, Ordering::Relaxed);
                return;
            }

            let elapsed = start.elapsed().as_secs();
            if elapsed >= last_log + 30 {
                last_log = elapsed;
                tracing::info!("Still waiting for LibreTranslate... ({}s)", elapsed);
            }

            if start.elapsed() > max_wait {
                tracing::error!(
                    "LibreTranslate did not become ready within {}s",
                    max_wait.as_secs()
                );
                status.store(SERVER_FAILED, Ordering::Relaxed);
                return;
            }
        }
    });
}
