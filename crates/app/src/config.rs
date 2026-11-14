//! The config file: which folders to index and which Bibles to load.
//!
//! ```toml
//! # Settings come before the first [[table]], or TOML files them under it.
//! brave_api_key = "…"   # optional, for !gi image search
//! colors = "~/.config/amane/colors.json"  # optional, see crate::palette
//! stage_output = "HDMI-A-1"  # optional: the screen for the stage
//!
//! [[folders]]
//! path = "~/Videos"
//! label = "Videos"      # optional
//! bang = "v"            # optional: !v searches only this folder
//!
//! [[bibles]]
//! lang = "es"           # "es" or "en": the language of the book names
//! path = "~/bibles/rv1909.json"
//! name = "RV1909"       # optional, defaults to the file name
//! ```
//!
//! `~` is the home folder, and a relative path is relative to the config
//! file's folder. Unknown keys are errors, so a typo doesn't silently drop a
//! folder. The settings view writes the file back with [`Config::save`].

use lantern_engine::Look;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use lantern_source_bible::Language;
use serde::{Deserialize, Serialize};

/// What the app indexes, with every path absolute or as written. Settings
/// come before the tables, which is the order TOML needs them in.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The Brave Search API key that `!gi` image search needs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brave_api_key: Option<String>,
    /// A JSON file of Material colour roles to take the dashboard's colours
    /// from, such as one Matugen writes from the wallpaper.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<PathBuf>,
    /// The screen the stage goes on, by its output name (`HDMI-A-1`).
    /// Unset, the compositor places it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_output: Option<String>,
    /// How see-through the dashboard is, in percent: 0 is solid, and the
    /// most is [`MOST_SEE_THROUGH`], so what's written on it still reads.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub see_through: u8,
    /// Don't ask the desktop to blur what shows through. Which desktops
    /// can blur, and how strongly, is theirs to say.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_blur: bool,
    #[serde(default)]
    pub folders: Vec<FolderEntry>,
    #[serde(default)]
    pub bibles: Vec<BibleEntry>,
    /// How the stage looks, as the `[stage]` table.
    #[serde(default, skip_serializing_if = "Look::is_default")]
    pub stage: Look,
    /// Keys that aren't the usual ones, as the `[keys]` table: an action's
    /// name to its key (`new = "ctrl+t"`). See `keys::ACTIONS`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, String>,
}

/// The most see-through the dashboard gets, in percent.
pub const MOST_SEE_THROUGH: u8 = 80;

fn is_zero(n: &u8) -> bool {
    *n == 0
}

/// A media folder to index.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FolderEntry {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Typed after `!` to search only this folder (and any other folder
    /// with the same bang). It still answers `!f` with every folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bang: Option<String>,
}

/// The bangs Lantern's own sources answer, which a folder can't take.
pub const TAKEN_BANGS: [&str; 4] = ["b", "f", "gi", "yt"];

/// A folder's bang as typed (`h`, `!H`), lowercased without its `!`, or
/// why it can't be one.
pub fn folder_bang(text: &str) -> Result<String, String> {
    let bang = text.trim().trim_start_matches('!').to_lowercase();
    if bang.is_empty() || bang.len() > 8 || !bang.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "“!{bang}” can't be a bang: use 1 to 8 letters or digits, like !h."
        ));
    }
    if TAKEN_BANGS.contains(&bang.as_str()) {
        return Err(format!(
            "!{bang} is taken by Lantern's own search: pick another."
        ));
    }
    Ok(bang)
}

/// A Bible translation file, `.json` or `.tsv`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BibleEntry {
    pub lang: Lang,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The language of a Bible's book names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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
        for path in config.paths_mut() {
            *path = resolve(path, dir, home);
        }
        Ok(config)
    }
}

impl Config {
    /// Every path in the config, to rewrite them all one way.
    fn paths_mut(&mut self) -> impl Iterator<Item = &mut PathBuf> {
        self.folders
            .iter_mut()
            .map(|f| &mut f.path)
            .chain(self.bibles.iter_mut().map(|b| &mut b.path))
            .chain(self.colors.as_mut())
            .chain(self.stage.image.as_mut())
    }

    /// The file's text, with paths under `home` written as `~/…`.
    pub fn to_toml(&self, home: Option<&Path>) -> String {
        let mut written = self.clone();
        if let Some(home) = home {
            for path in written.paths_mut() {
                *path = tilde(path, home);
            }
        }
        let body = toml::to_string(&written).expect("a config always serializes");
        format!("# Lantern's config, written by its settings (Ctrl+,).\n\n{body}")
    }

    /// Writes the file, through a temporary one so a crash never leaves
    /// half a config.
    pub fn save(&self, path: &Path, home: Option<&Path>) -> Result<(), String> {
        let fail = |err: std::io::Error| format!("{}: {err}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let partial = path.with_extension("toml.part");
        std::fs::write(&partial, self.to_toml(home)).map_err(fail)?;
        std::fs::rename(&partial, path).map_err(fail)
    }
}

/// `path` with `home` written as `~`, the way people type it.
pub fn tilde(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix(home) {
        Ok(rest) => Path::new("~").join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// `path` as the config reads it: `~` is `home`, and relative is relative
/// to `dir`.
pub fn resolve(path: &Path, dir: &Path, home: Option<&Path>) -> PathBuf {
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

impl Status {
    /// The file, read or not.
    pub fn path(&self) -> &Path {
        match self {
            Status::Read(path) | Status::Missing(path) | Status::Failed { path, .. } => path,
        }
    }

    /// What the dashboard should say about the config: only a failure.
    pub fn problem(&self) -> Option<String> {
        matches!(self, Status::Failed { .. }).then(|| self.to_string())
    }
}

/// Reads `$LANTERN_CONFIG`, else `config.toml` in [`xdg_dir`] for
/// `XDG_CONFIG_HOME` (`~/.config/lantern`). Never fails: a broken file
/// yields the default config and a [`Status::Failed`].
pub fn load(env: impl Fn(&str) -> Option<String>) -> (Config, Status) {
    let home = env("HOME").map(PathBuf::from);
    let named = env("LANTERN_CONFIG").map(PathBuf::from);
    let path = match &named {
        Some(path) => path.clone(),
        None => xdg_dir(&env, "XDG_CONFIG_HOME", ".config").join("config.toml"),
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

/// The process environment, for [`load`] and [`xdg_dir`].
pub fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Lantern's folder in an XDG base directory: `$var/lantern`, else
/// `~/fallback/lantern`. An empty variable counts as unset.
pub fn xdg_dir(env: impl Fn(&str) -> Option<String>, var: &str, fallback: &str) -> PathBuf {
    match (env(var), env("HOME")) {
        (Some(dir), _) if !dir.is_empty() => PathBuf::from(dir),
        (_, Some(home)) => Path::new(&home).join(fallback),
        (_, None) => PathBuf::from(fallback),
    }
    .join("lantern")
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
            colors = "~/.config/amane/colors.json"
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
        assert_eq!(
            config.colors,
            Some(PathBuf::from("/home/ana/.config/amane/colors.json"))
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
            Status::Missing(Path::new(home).join(".config/lantern/config.toml"))
        );

        let (_, status) = load_with(&[("HOME", home), ("XDG_CONFIG_HOME", "/nowhere")]);
        assert_eq!(
            status,
            Status::Missing("/nowhere/lantern/config.toml".into())
        );

        let (config, status) = load_with(&[("LANTERN_CONFIG", "/nowhere/cq.toml")]);
        assert_eq!(config, Config::default());
        assert!(matches!(status, Status::Failed { .. }), "{status:?}");
    }

    #[test]
    fn a_malformed_file_yields_defaults_and_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[folders]]\npath = \"media\"\n").unwrap();
        let named = path.to_str().unwrap();
        let (config, status) = load_with(&[("LANTERN_CONFIG", named)]);
        assert_eq!(config.folders[0].path, dir.path().join("media"));
        assert_eq!(status, Status::Read(path.clone()));

        std::fs::write(&path, "[[folders]\n").unwrap();
        let (config, status) = load_with(&[("LANTERN_CONFIG", named)]);
        assert_eq!(config, Config::default());
        let Status::Failed { error, .. } = status else {
            panic!("{status:?}")
        };
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn a_saved_config_reads_back_the_same_with_home_as_a_tilde() {
        let home = Path::new("/home/ana");
        let config = Config {
            brave_api_key: Some("key".into()),
            colors: Some("/home/ana/.config/amane/colors.json".into()),
            stage_output: Some("HDMI-A-1".into()),
            see_through: 40,
            no_blur: true,
            keys: [("new".to_string(), "ctrl+t".to_string())].into(),
            folders: vec![FolderEntry {
                path: "/home/ana/Music".into(),
                label: Some("Music".into()),
                bang: Some("m".into()),
            }],
            bibles: vec![BibleEntry {
                lang: Lang::Es,
                path: "/srv/bibles/rv1909.tsv".into(),
                name: None,
            }],
            stage: Look {
                image: Some("/home/ana/Pictures/cross.jpg".into()),
                font: Some("DejaVu Serif".into()),
                ..Look::default()
            },
        };
        let text = config.to_toml(Some(home));
        assert!(text.contains("image = \"~/Pictures/cross.jpg\""), "{text}");
        assert!(text.contains("path = \"~/Music\""), "{text}");
        assert!(text.contains("/srv/bibles/rv1909.tsv"), "{text}");
        assert!(
            text.contains("see_through = 40") && text.contains("no_blur = true"),
            "{text}"
        );
        // Solid and blurred are the usual: a plain config says neither.
        let plain = Config::default().to_toml(Some(home));
        assert!(
            !plain.contains("see_through") && !plain.contains("blur"),
            "{plain}"
        );
        let settings = text.find("colors").unwrap();
        assert!(
            settings < text.find("[[folders]]").unwrap(),
            "settings first: {text}"
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lantern/config.toml");
        config.save(&path, Some(home)).unwrap();
        let back = Config::parse(
            &std::fs::read_to_string(&path).unwrap(),
            dir.path(),
            Some(home),
        );
        assert_eq!(back.unwrap(), config);
    }
}
