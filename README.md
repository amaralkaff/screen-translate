<p align="center">
  <img src="assets/logo.png" width="120" />
</p>

<h1 align="center">Screen Translate</h1>

<p align="center">select text anywhere. get instant translation. all local.</p>

---

<p align="center">highlight text with your mouse and a popup appears with the translation. works with any app. everything runs on your machine via libretranslate.</p>

## installation

### Option 1: Bundled Installer (Recommended)

Download the latest release from [GitHub Releases](https://github.com/amaralkaff/screen-translate/releases/latest):

**Windows:**
- Download `ScreenTranslate-Full-x.x.x-setup.exe`
- Run the installer (includes bundled Python 3.12 + LibreTranslate)
- Auto-starts on login via registry
- Everything works offline immediately

**macOS:**
- Download `ScreenTranslate-x.x.x-macOS.dmg`
- Drag `Screen Translate.app` to Applications
- Grant **Input Monitoring** permission (System Settings > Privacy & Security)
- First launch downloads language models (~2-5 min)

**Enable auto-start on macOS:**
```bash
cp '/Applications/Screen Translate.app/Contents/Resources/com.amaralkaff.screen-translate.plist' \
   ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.amaralkaff.screen-translate.plist
```

### Option 2: Manual Build (Advanced)

#### 1. install libretranslate

```bash
# Windows
py -3.12 -m venv ../libretranslate
../libretranslate/Scripts/pip install libretranslate
../libretranslate/Scripts/pip install 'setuptools==67.8.0'

# macOS
python3.12 -m venv ../libretranslate
../libretranslate/bin/pip install libretranslate
../libretranslate/bin/pip install 'setuptools==67.8.0'
```

> ⚠️ **IMPORTANT:** Use **Python 3.12 only**. Python 3.13+ and 3.14 are not compatible. Setuptools 67.8.0 required for pkg_resources compatibility.

#### 2. build and run

```bash
cargo build --release
cargo run --release
```

auto-starts libretranslate if not running. first launch downloads language models (~2-5 min). subsequent launches are fast.

**macOS note:** on first launch, macOS will prompt for **Input Monitoring** permission. grant it in System Settings > Privacy & Security > Input Monitoring, then relaunch.

#### 3. configure (optional)

config location:
- **macOS:** `~/Library/Application Support/screen-translate/config.toml`
- **Windows:** `%APPDATA%/screen-translate/config.toml`

```toml
target_lang = "id"
source_lang = "auto"
poll_interval_ms = 300
load_languages = "en,id"
```

> fewer languages = faster startup. only load what you need.

see `config.example.toml` for all options.

## how it works

```
select text → copy simulated (Cmd+C / Ctrl+C) → libretranslate api → popup appears above selection
```

- **Windows:** Win32 mouse hook + layered window with GDI rendering
- **macOS:** CGEventTap + NSPanel with vibrancy (Liquid Glass on macOS 26+)

## troubleshooting

**libretranslate crashes on startup / flickering windows:** recreate venv with Python 3.12 and correct setuptools:
```bash
py -3.12 -m venv ../libretranslate
../libretranslate/Scripts/pip install libretranslate 'setuptools==67.8.0'
```

**libretranslate won't start:** make sure venv exists at `../libretranslate/` relative to the exe.

**macOS port 5000 conflict:** macOS uses port 5000 for AirPlay. screen-translate defaults to port 5001 on macOS.

**macOS permission denied:** enable Input Monitoring in System Settings > Privacy & Security > Input Monitoring.

**no popup:** check tray icon — monitoring might be toggled off.

**slow first run:** models are downloading. reduce `load_languages` to speed things up (e.g. `"en,id"`).

## roadmap

- [x] windows support
- [x] macos support (CGEventTap + NSPanel)