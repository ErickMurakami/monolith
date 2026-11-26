use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: Theme,
    pub language: Language,
    pub shortcut: Shortcut,
    pub win_space_override: bool,
    pub dock: bool,
    pub dock_pins: Vec<String>,
    pub dock_monitor: DockMonitor,
    pub dock_auto_hide: bool,
    pub dock_original_icons: bool,
    pub top_bar: bool,
    pub island: bool,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DockMonitor {
    #[default]
    Primary,
    Active,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    English,
    Portuguese,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shortcut {
    #[default]
    WinSpace,
    CtrlAltSpace,
    Disabled,
}

pub fn load(path: &Path, safe: bool) -> (Config, Option<String>) {
    if safe {
        return (
            Config {
                shortcut: Shortcut::Disabled,
                ..Config::default()
            },
            None,
        );
    }
    let parsed = match fs::read_to_string(path) {
        Ok(source) => toml::from_str(&source).map_err(|error| {
            // The original configuration may contain private data.
            error.span().map_or("Invalid configuration".into(), |span| {
                format!("Invalid configuration at byte {}", span.start)
            })
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(format!("Could not read configuration ({})", error.kind())),
    };
    match parsed {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error)),
    }
}

#[derive(Deserialize, Serialize)]
struct DockOrder {
    pins: Vec<String>,
}

pub fn load_dock_order(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|source| toml::from_str::<DockOrder>(&source).ok())
        .map_or_else(Vec::new, |order| order.pins)
}

pub fn save_dock_order(path: &Path, pins: Vec<String>) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid Dock order path"))?;
    let source = toml::to_string(&DockOrder { pins }).map_err(io::Error::other)?;
    fs::create_dir_all(parent)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let temp = parent.join(format!(
        ".dock-order-{}-{}.tmp",
        std::process::id(),
        stamp.as_nanos()
    ));
    let mut file = fs::File::create_new(&temp)?;
    let written = file
        .write_all(source.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_configuration_is_preserved() -> io::Result<()> {
        let source = "theme = 'invalid'";
        let path =
            std::env::temp_dir().join(format!("monolith-config-{}.toml", std::process::id()));
        fs::write(&path, source)?;
        let (config, issue) = load(&path, false);
        let original = fs::read_to_string(&path);
        fs::remove_file(path)?;
        assert_eq!(original?, source);
        assert!(matches!(config.theme, Theme::Dark));
        assert!(issue.is_some());
        Ok(())
    }

    #[test]
    fn safe_mode_ignores_configuration_and_disables_shortcut() {
        let (config, issue) = load(Path::new(""), true);
        assert!(matches!(config.shortcut, Shortcut::Disabled));
        assert!(!config.dock);
        assert!(!config.top_bar);
        assert!(!config.island);
        assert!(issue.is_none());
    }

    #[test]
    fn replaces_the_previous_order_in_a_separate_file() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("monolith-pin-order-{}", std::process::id()));
        let order = root.join("dock-order.toml");
        save_dock_order(&order, vec!["First".into(), "Second".into()])?;
        save_dock_order(&order, vec!["Second".into(), "First".into()])?;
        let saved = load_dock_order(&order);
        fs::remove_file(order)?;
        fs::remove_dir(root)?;
        assert_eq!(saved, ["Second", "First"]);
        Ok(())
    }
}
