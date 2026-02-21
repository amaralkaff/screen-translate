use anyhow::Result;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

fn lang_display_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "id" => "Indonesian",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "es" => "Spanish",
        "ar" => "Arabic",
        "fr" => "French",
        "de" => "German",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "ko" => "Korean",
        "it" => "Italian",
        "hi" => "Hindi",
        "tr" => "Turkish",
        _ => code,
    }
}

pub struct Tray {
    _tray: TrayIcon,
    pub monitor_item: CheckMenuItem,
    quit_id: MenuId,
    lang_items: Vec<(CheckMenuItem, String)>,
    update_item: MenuItem,
}

impl Tray {
    pub fn new(languages: &[String], current_lang: &str) -> Result<Self> {
        let menu = Menu::new();
        let monitor_item = CheckMenuItem::new("Monitoring Active", true, true, None);
        menu.append(&monitor_item)?;

        let lang_submenu = Submenu::new("Target Language", true);
        let mut lang_items = Vec::new();
        for code in languages {
            let label = format!("{} ({})", lang_display_name(code), code);
            let checked = code == current_lang;
            let item = CheckMenuItem::new(label, true, checked, None);
            lang_submenu.append(&item)?;
            lang_items.push((item, code.clone()));
        }
        menu.append(&lang_submenu)?;

        let update_item = MenuItem::new("Check for Updates", true, None);
        menu.append(&update_item)?;

        menu.append(&PredefinedMenuItem::separator())?;

        let quit_item = MenuItem::new("Quit", true, None);
        let quit_id = quit_item.id().clone();
        menu.append(&quit_item)?;

        let icon = load_default_icon()?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("Screen Translate")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()?;

        Ok(Self {
            _tray: tray,
            monitor_item,
            quit_id,
            lang_items,
            update_item,
        })
    }

    pub fn handle_menu_event(&self) -> TrayAction {
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if *event.id() == self.quit_id {
                return TrayAction::Quit;
            }

            if *event.id() == *self.update_item.id() {
                return TrayAction::CheckForUpdates;
            }

            for (item, code) in &self.lang_items {
                if *event.id() == *item.id() {
                    // Uncheck all, check the selected one
                    for (other, _) in &self.lang_items {
                        other.set_checked(false);
                    }
                    item.set_checked(true);
                    return TrayAction::ChangeLanguage(code.clone());
                }
            }

            return TrayAction::ToggleMonitoring(self.monitor_item.is_checked());
        }
        TrayAction::None
    }

    pub fn set_update_in_progress(&self) {
        self.update_item.set_text("Updating...");
        self.update_item.set_enabled(false);
    }
}

fn load_default_icon() -> Result<Icon> {
    let png_bytes = include_bytes!("../assets/logo.png");
    let img = image::load_from_memory(png_bytes)?
        .resize(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok(Icon::from_rgba(img.into_raw(), w, h)?)
}

pub enum TrayAction {
    None,
    Quit,
    ToggleMonitoring(bool),
    ChangeLanguage(String),
    CheckForUpdates,
}
