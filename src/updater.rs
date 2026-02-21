use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

const GITHUB_API_URL: &str =
    "https://api.github.com/repos/amaralkaff/screen-translate/releases/latest";

#[derive(Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: String,
    pub asset_name: String,
    strategy: UpdateStrategy,
}

#[derive(Clone)]
enum UpdateStrategy {
    InstallerSilent, // Windows installed via Inno Setup
    #[allow(dead_code)] // Only constructed on macOS
    DmgInstall, // macOS .app bundle
    BinarySwap,      // macOS standalone or Windows standalone
}

fn current_version() -> (u64, u64, u64) {
    let v = env!("CARGO_PKG_VERSION");
    parse_version(v).unwrap_or((0, 0, 0))
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "screen-translate.exe"
    } else {
        "screen-translate"
    }
}

fn build_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("screen-translate/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")
}

/// Detect whether this is an Inno Setup installation by checking for unins000.exe.
fn is_inno_setup_install() -> bool {
    if !cfg!(target_os = "windows") {
        return false;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("unins000.exe").exists();
        }
    }
    false
}

/// Detect if running inside a macOS `.app` bundle (`Contents/MacOS/`).
fn is_app_bundle() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            return macos_dir.ends_with("Contents/MacOS");
        }
    }
    false
}

/// Resolve the `.app` bundle root directory from the current exe path.
/// e.g. `/Applications/Screen Translate.app/Contents/MacOS/screen-translate`
///   -> `/Applications/Screen Translate.app`
#[cfg(target_os = "macos")]
fn find_app_bundle_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // exe -> Contents/MacOS/ -> Contents/ -> Screen Translate.app
    let app_dir = exe.parent()?.parent()?.parent()?;
    if app_dir.extension().and_then(|e| e.to_str()) == Some("app") {
        Some(app_dir.to_path_buf())
    } else {
        None
    }
}

/// Check if the `.app` bundle contains the LibreTranslate Python environment.
#[cfg(target_os = "macos")]
fn app_bundle_has_libretranslate() -> bool {
    if let Some(app_root) = find_app_bundle_root() {
        app_root
            .join("Contents/Resources/libretranslate/bin/python3")
            .exists()
    } else {
        false
    }
}

/// Remove `<current_exe>.old` left over from a previous update.
/// On macOS, also removes `.app.old` directories from full DMG updates.
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let old = exe.with_extension("old");
        if old.exists() {
            if let Err(e) = std::fs::remove_file(&old) {
                tracing::debug!("Failed to remove old binary: {}", e);
            } else {
                tracing::info!("Cleaned up old binary");
            }
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(app_root) = find_app_bundle_root() {
        let app_old = app_root.with_extension("app.old");
        if app_old.exists() {
            if let Err(e) = std::fs::remove_dir_all(&app_old) {
                tracing::debug!("Failed to remove old .app bundle: {}", e);
            } else {
                tracing::info!("Cleaned up old .app bundle");
            }
        }
    }
}

/// Check GitHub for a newer release. Returns update info if available.
pub fn check_for_update() -> Result<Option<UpdateInfo>> {
    let current = current_version();
    tracing::info!(
        "Checking for updates... (current v{}.{}.{})",
        current.0,
        current.1,
        current.2
    );

    let client = build_client()?;

    let resp: serde_json::Value = client
        .get(GITHUB_API_URL)
        .send()
        .context("Failed to reach GitHub API")?
        .error_for_status()
        .context("GitHub API returned error")?
        .json()
        .context("Failed to parse release JSON")?;

    let tag = resp["tag_name"]
        .as_str()
        .context("Missing tag_name in release")?;

    let remote = parse_version(tag).context("Cannot parse remote version")?;

    if remote <= current {
        tracing::info!("Up to date (v{}.{}.{})", current.0, current.1, current.2);
        return Ok(None);
    }

    tracing::info!(
        "New version available: {} (current v{}.{}.{})",
        tag,
        current.0,
        current.1,
        current.2
    );

    let version = tag.strip_prefix('v').unwrap_or(tag).to_string();

    // Determine update strategy and find the right asset
    let (strategy, asset_name) = if is_inno_setup_install() {
        let installer_name = format!("ScreenTranslate-{}-setup.exe", version);
        if find_asset_url(&resp, &installer_name).is_ok() {
            (UpdateStrategy::InstallerSilent, installer_name)
        } else {
            tracing::info!("Installer asset not found, falling back to binary zip");
            (
                UpdateStrategy::BinarySwap,
                "screen-translate-windows-x64.zip".to_string(),
            )
        }
    } else if cfg!(target_os = "macos") && is_app_bundle() {
        #[cfg(target_os = "macos")]
        {
            if app_bundle_has_libretranslate() {
                // Full bundle exists — just update the binary (fast, ~20MB)
                (
                    UpdateStrategy::DmgInstall,
                    "screen-translate-macos-arm64.zip".to_string(),
                )
            } else {
                // .app bundle but no LibreTranslate — need the full DMG
                let dmg_name = format!("ScreenTranslate-{}.dmg", version);
                if find_asset_url(&resp, &dmg_name).is_ok() {
                    (UpdateStrategy::DmgInstall, dmg_name)
                } else {
                    tracing::info!("DMG asset not found, falling back to binary zip");
                    (
                        UpdateStrategy::DmgInstall,
                        "screen-translate-macos-arm64.zip".to_string(),
                    )
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            unreachable!("is_app_bundle() returned true on non-macOS")
        }
    } else if cfg!(target_os = "macos") {
        // Standalone binary (not inside .app), use binary swap
        (
            UpdateStrategy::BinarySwap,
            "screen-translate-macos-arm64.zip".to_string(),
        )
    } else {
        (
            UpdateStrategy::BinarySwap,
            "screen-translate-windows-x64.zip".to_string(),
        )
    };

    let download_url = find_asset_url(&resp, &asset_name)?;

    Ok(Some(UpdateInfo {
        version,
        download_url,
        asset_name,
        strategy,
    }))
}

/// Download and apply the update.
pub fn perform_update(info: &UpdateInfo) -> Result<()> {
    // Use a longer timeout for potentially large DMG downloads
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("screen-translate/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .context("Failed to build HTTP client")?;

    let tmp_dir = tempdir()?;

    tracing::info!("Downloading {}...", info.asset_name);
    let bytes = client
        .get(&info.download_url)
        .send()
        .context("Failed to download asset")?
        .error_for_status()
        .context("Asset download returned error")?
        .bytes()
        .context("Failed to read asset bytes")?;

    let download_path = tmp_dir.join(&info.asset_name);
    std::fs::write(&download_path, &bytes).context("Failed to write download to temp dir")?;

    match info.strategy {
        UpdateStrategy::InstallerSilent => {
            tracing::info!("Launching silent installer...");
            Command::new(&download_path)
                .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
                .spawn()
                .context("Failed to launch installer")?;
            std::process::exit(0);
        }
        UpdateStrategy::DmgInstall => {
            #[cfg(target_os = "macos")]
            {
                let app_root =
                    find_app_bundle_root().context("Cannot determine .app bundle root")?;

                if info.asset_name.ends_with(".dmg") {
                    perform_dmg_update(&download_path, &app_root)?;
                } else {
                    perform_app_binary_swap(&download_path, &tmp_dir, &app_root)?;
                }
                // Unreachable: both functions call exit(0) on success
                Ok(())
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = &download_path;
                bail!("DmgInstall strategy is only supported on macOS");
            }
        }
        UpdateStrategy::BinarySwap => {
            tracing::info!("Extracting...");
            extract_binary(&download_path, &tmp_dir)?;

            let extracted = tmp_dir.join(binary_name());
            if !extracted.exists() {
                bail!("Extracted binary not found at {}", extracted.display());
            }

            let current_exe =
                std::env::current_exe().context("Cannot determine current exe path")?;
            let old_exe = current_exe.with_extension("old");

            std::fs::rename(&current_exe, &old_exe)
                .context("Failed to rename current exe to .old")?;

            std::fs::copy(&extracted, &current_exe)
                .context("Failed to copy new binary into place")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &current_exe,
                    std::fs::Permissions::from_mode(0o755),
                )
                .context("Failed to set executable permission")?;
            }

            tracing::info!("Update applied — relaunching...");

            Command::new(&current_exe)
                .spawn()
                .context("Failed to relaunch after update")?;

            std::process::exit(0);
        }
    }
}

/// Full DMG update: mount DMG, copy .app bundle, relaunch.
#[cfg(target_os = "macos")]
fn perform_dmg_update(dmg_path: &PathBuf, app_root: &PathBuf) -> Result<()> {
    tracing::info!("Mounting DMG...");
    let output = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-noverify", "-noautoopen"])
        .arg(dmg_path)
        .output()
        .context("Failed to run hdiutil attach")?;

    if !output.status.success() {
        bail!(
            "hdiutil attach failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Parse mount point from hdiutil stdout (last column of last line)
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mount_point = stdout
        .lines()
        .last()
        .and_then(|line| line.split('\t').next_back())
        .map(|s| s.trim())
        .context("Cannot parse mount point from hdiutil output")?
        .to_string();

    tracing::info!("DMG mounted at {}", mount_point);

    // Find the .app inside the mounted volume
    let mounted_app = PathBuf::from(&mount_point).join("Screen Translate.app");
    if !mounted_app.exists() {
        let _ = Command::new("hdiutil")
            .args(["detach", &mount_point])
            .status();
        bail!(
            "Screen Translate.app not found in DMG at {}",
            mounted_app.display()
        );
    }

    // Remove any leftover .app.old, then rename current .app -> .app.old
    let app_old = app_root.with_extension("app.old");
    if app_old.exists() {
        std::fs::remove_dir_all(&app_old)
            .context("Failed to remove leftover .app.old directory")?;
    }

    tracing::info!("Backing up current .app bundle...");
    std::fs::rename(app_root, &app_old).context("Failed to rename current .app to .app.old")?;

    // Use ditto to copy preserving macOS metadata and signatures
    tracing::info!("Copying new .app bundle...");
    let ditto_status = Command::new("ditto")
        .arg(&mounted_app)
        .arg(app_root)
        .status()
        .context("Failed to run ditto")?;

    if !ditto_status.success() {
        // Recovery: rename .app.old back
        tracing::error!("ditto failed, recovering from backup...");
        if let Err(e) = std::fs::rename(&app_old, app_root) {
            tracing::error!("Recovery failed: {}. Manual intervention needed.", e);
        }
        let _ = Command::new("hdiutil")
            .args(["detach", &mount_point])
            .status();
        bail!("ditto failed to copy .app bundle");
    }

    // Unmount DMG (best-effort)
    let _ = Command::new("hdiutil")
        .args(["detach", &mount_point])
        .status();

    tracing::info!("DMG update applied — relaunching...");
    Command::new("open")
        .arg("-a")
        .arg(app_root)
        .spawn()
        .context("Failed to relaunch .app after DMG update")?;

    std::process::exit(0);
}

/// Binary-only update for .app bundles: extract binary from zip, swap in place, relaunch via `open`.
#[cfg(target_os = "macos")]
fn perform_app_binary_swap(
    zip_path: &PathBuf,
    tmp_dir: &PathBuf,
    app_root: &PathBuf,
) -> Result<()> {
    tracing::info!("Extracting binary for .app bundle update...");
    extract_binary(zip_path, tmp_dir)?;

    let extracted = tmp_dir.join(binary_name());
    if !extracted.exists() {
        bail!("Extracted binary not found at {}", extracted.display());
    }

    let current_exe = std::env::current_exe().context("Cannot determine current exe path")?;
    let old_exe = current_exe.with_extension("old");

    std::fs::rename(&current_exe, &old_exe).context("Failed to rename current exe to .old")?;

    std::fs::copy(&extracted, &current_exe).context("Failed to copy new binary into place")?;

    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755))
            .context("Failed to set executable permission")?;
    }

    tracing::info!("Binary update applied — relaunching .app...");
    Command::new("open")
        .arg("-a")
        .arg(app_root)
        .spawn()
        .context("Failed to relaunch .app after binary update")?;

    std::process::exit(0);
}

fn find_asset_url(release: &serde_json::Value, asset_name: &str) -> Result<String> {
    let assets = release["assets"]
        .as_array()
        .context("Missing assets array")?;

    for asset in assets {
        if asset["name"].as_str() == Some(asset_name) {
            return asset["browser_download_url"]
                .as_str()
                .map(String::from)
                .context("Missing browser_download_url");
        }
    }
    bail!("Asset '{}' not found in release", asset_name);
}

fn tempdir() -> Result<PathBuf> {
    let dir =
        std::env::temp_dir().join(format!("screen-translate-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir).context("Failed to create temp dir")?;
    Ok(dir)
}

fn extract_binary(zip_path: &PathBuf, out_dir: &PathBuf) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("unzip")
            .args(["-o", "-j"])
            .arg(zip_path)
            .arg(binary_name())
            .arg("-d")
            .arg(out_dir)
            .status()
            .context("Failed to run unzip")?;
        if !status.success() {
            bail!("unzip exited with {}", status);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("tar")
            .args(["-xf"])
            .arg(zip_path)
            .arg("-C")
            .arg(out_dir)
            .arg(binary_name())
            .status()
            .context("Failed to run tar")?;
        if !status.success() {
            bail!("tar exited with {}", status);
        }
    }

    Ok(())
}
