//! The config file: which folders to index and which Bibles to load.
//!
//! ```toml
//! [[folders]]
//! path = "~/Videos"
//! label = "Videos"      # optional
//!
//! [[bibles]]
//! lang = "es"           # "es" or "en": the language of the book names
//! path = "~/bibles/rv1909.json"
//! name = "RV1909"       # optional, defaults to the file name
//! ```
//!
//! `~` is the home folder, and a relative path is relative to the config
//! file's folder. Unknown keys are errors, so a typo doesn't silently drop a
//! folder.

use std::fmt;
use std::path::{Path, PathBuf};

use chorquest_source_bible::Language;
use serde::Deserialize;

/// What the app indexes, with every path absolute or as written.
#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub folders: Vec<FolderEntry>,
    #[serde(default)]
    pub bibles: Vec<BibleEntry>,
}

/// A media folder to index.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderEntry {
    pub path: PathBuf,
    pub label: Option<String>,
}

/// A Bible translation file, `.json` or `.tsv`.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BibleEntry {
    pub lang: Lang,
    pub path: PathBuf,
    pub name: Option<String>,
}

/// The language of a Bible's book names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Es,
    En,
}

impl From<Lang> for Language {
    fn from(lang: Lang) -> Language {
        match lang {
            Lang::Es => Language::Spanish,
            Lang::En => Language::English,
        }
    }
}

impl Config {
    /// Parses a config file's text. Relative paths are resolved against
    /// `dir`, and `~` against `home`.
    pub fn parse(source: &str, dir: &Path, home: Option<&Path>) -> Result<Config, String> {
        let mut config: Config =
            toml::from_str(source).map_err(|err| err.to_string().trim_end().to_string())?;
        let paths = config
            .folders
            .iter_mut()
            .map(|f| &mut f.path)
            .chain(config.bibles.iter_mut().map(|b| &mut b.path));
        for path in paths {
            *path = resolve(path, dir, home);
        }
        Ok(config)
    }
}

fn resolve(path: &Path, dir: &Path, home: Option<&Path>) -> PathBuf {
    let path = match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    };
    dir.join(path)
}

/// Where the config came from, for the dashboard.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Read(PathBuf),
    /// The default file doesn't exist, which is fine.
    Missing(PathBuf),
    /// The file couldn't be read or parsed, so the app runs with defaults.
    Failed {
        path: PathBuf,
        error: String,
    },
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Read(path) => write!(f, "Config: {}", path.display()),
            Status::Missing(path) => write!(
                f,
                "Config: no file at {}, so no folders or Bibles",
                path.display()
            ),
            Status::Failed { path, error } => write!(
                f,
                "Config error in {}, running with no folders or Bibles: {error}",
                path.display()
            ),
        }
    }
}

/// Reads `$CHORQUEST_CONFIG`, else `$XDG_CONFIG_HOME/chorquest/config.toml`,
/// else `~/.config/chorquest/config.toml`. Never fails: a broken file
/// yields the default config and a [`Status::Failed`].
pub fn load(env: impl Fn(&str) -> Option<String>) -> (Config, Status) {
    let home = env("HOME").map(PathBuf::from);
    let named = env("CHORQUEST_CONFIG").map(PathBuf::from);
    let path = match &named {
        Some(path) => path.clone(),
        None => match (env("XDG_CONFIG_HOME"), &home) {
            (Some(xdg), _) if !xdg.is_empty() => PathBuf::from(xdg),
            (_, Some(home)) => home.join(".config"),
            (_, None) => PathBuf::from(".config"),
        }
        .join("chorquest/config.toml"),
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && named.is_none() => {
            return (Config::default(), Status::Missing(path));
        }
        Err(err) => {
            let error = err.to_string();
            return (Config::default(), Status::Failed { path, error });
        }
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    match Config::parse(&source, dir, home.as_deref()) {
        Ok(config) => (config, Status::Read(path)),
        Err(error) => (Config::default(), Status::Failed { path, error }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn parse(source: &str) -> Result<Config, String> {
        Config::parse(source, Path::new("/etc/cq"), Some(Path::new("/home/ana")))
    }

    #[test]
    fn paths_expand_tilde_and_resolve_against_the_config_folder() {
        let config = parse(
            r#"
            [[folders]]
            path = "~/Videos"
            label = "Videos"
            [[folders]]
            path = "media"
            [[folders]]
            path = "/srv/pictures"
            [[bibles]]
            lang = "es"
            path = "../bibles/rv1909.json"
            name = "RV1909"
            "#,
        )
        .unwrap();
        let folders: Vec<_> = config.folders.iter().map(|f| f.path.clone()).collect();
        assert_eq!(
            folders,
            [
                PathBuf::from("/home/ana/Videos"),
                PathBuf::from("/etc/cq/media"),
                PathBuf::from("/srv/pictures"),
            ]
        );
        assert_eq!(config.folders[0].label.as_deref(), Some("Videos"));
        assert_eq!(config.folders[1].label, None);
        assert_eq!(
            config.bibles,
            [BibleEntry {
                lang: Lang::Es,
                path: "/etc/cq/../bibles/rv1909.json".into(),
                name: Some("RV1909".into()),
            }]
        );
    }

    #[test]
    fn an_empty_file_is_the_default() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn unknown_keys_and_languages_are_errors() {
        let err = parse("[[folders]]\npath = \"a\"\nlabl = \"A\"").unwrap_err();
        assert!(err.contains("unknown field `labl`"), "{err}");
        let err = parse("[[folder]]\npath = \"a\"").unwrap_err();
        assert!(err.contains("unknown field `folder`"), "{err}");
        let err = parse("[[bibles]]\nlang = \"fr\"\npath = \"a.tsv\"").unwrap_err();
        assert!(err.contains("unknown variant `fr`"), "{err}");
    }

    fn load_with(vars: &[(&str, &str)]) -> (Config, Status) {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        load(|key| vars.get(key).cloned())
    }

    #[test]
    fn a_missing_default_file_is_fine_but_a_missing_named_file_is_not() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().to_str().unwrap();
        let (config, status) = load_with(&[("HOME", home)]);
        assert_eq!(config, Config::default());
        assert_eq!(
            status,
            Status::Missing(Path::new(home).join(".config/chorquest/config.toml"))
        );

        let (_, status) = load_with(&[("HOME", home), ("XDG_CONFIG_HOME", "/nowhere")]);
        assert_eq!(
            status,
            Status::Missing("/nowhere/chorquest/config.toml".into())
        );

        let (config, status) = load_with(&[("CHORQUEST_CONFIG", "/nowhere/cq.toml")]);
        assert_eq!(config, Config::default());
        assert!(matches!(status, Status::Failed { .. }), "{status:?}");
    }

    #[test]
    fn a_malformed_file_yields_defaults_and_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[folders]]\npath = \"media\"\n").unwrap();
        let named = path.to_str().unwrap();
        let (config, status) = load_with(&[("CHORQUEST_CONFIG", named)]);
        assert_eq!(config.folders[0].path, dir.path().join("media"));
        assert_eq!(status, Status::Read(path.clone()));

        std::fs::write(&path, "[[folders]\n").unwrap();
        let (config, status) = load_with(&[("CHORQUEST_CONFIG", named)]);
        assert_eq!(config, Config::default());
        let Status::Failed { error, .. } = status else {
            panic!("{status:?}")
        };
        assert!(error.contains("line 1"), "{error}");
    }
}
