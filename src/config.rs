use serde::Deserialize;
use std::{fs, io, path::Path};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: Theme,
    pub language: Language,
    pub shortcut: Shortcut,
    pub dock: bool,
    pub dock_pins: Vec<String>,
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
        assert!(issue.is_none());
    }
}
