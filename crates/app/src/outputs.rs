//! Which screen (output) the stage is on. iced can't list monitors or
//! fullscreen a window on a chosen one, and on Wayland only the compositor
//! places windows, so this asks the compositor. Only Hyprland so far:
//! `hyprctl` lists the outputs and moves the stage window (found by its app
//! id and this process, so another Lantern is never touched).
//!
//! Where the stage goes is decided here ([`choose`]): the screen the
//! settings prefer when it is connected, else the screen it was last put
//! on by hand with these same screens connected, else a screen the
//! dashboard isn't on. With one screen and no preference there is nowhere
//! for it, and the dashboard's live picture is the stage.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// A screen the compositor knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Its connector name, like `HDMI-A-1`: what the config stores.
    pub name: String,
    /// Its make and model, as the screen reports them.
    pub description: String,
    pub width: u32,
    pub height: u32,
}

impl Output {
    /// "HDMI-A-1 · Epson · 1920×1080".
    pub fn label(&self) -> String {
        let model = self.description.trim();
        let size = format!("{}×{}", self.width, self.height);
        if model.is_empty() {
            format!("{} · {size}", self.name)
        } else {
            format!("{} · {model} · {size}", self.name)
        }
    }
}

impl Output {
    /// What a screen is known by, wherever it is plugged in: its make and
    /// model. A screen that reports none goes by its connector.
    pub fn id(&self) -> &str {
        match self.description.trim() {
            "" => &self.name,
            model => model,
        }
    }

    /// Whether `wanted` means this screen: by make and model, or by
    /// connector, as configs written before screens had names do.
    pub fn is(&self, wanted: &str) -> bool {
        let wanted = wanted.trim();
        wanted == self.name || (!wanted.is_empty() && wanted == self.description.trim())
    }
}

/// The screens connected, as one name: what a remembered place is kept
/// under. The same screens in another order are the same setup.
pub fn setup(connected: &[Output]) -> String {
    let mut ids: Vec<&str> = connected.iter().map(Output::id).collect();
    ids.sort_unstable();
    ids.join(" + ")
}

/// Where the stage goes, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The screen the settings prefer is connected: the stage is on it
    /// from the moment it is plugged in, whatever the mode.
    Preferred(String),
    /// Where the stage was last put by hand with these screens.
    Remembered(String),
    /// Nothing says where: a screen the dashboard isn't on.
    Spare(String),
    /// One screen and no preference: no stage window. The dashboard's
    /// live picture shows what there is to see.
    Nowhere,
}

impl Place {
    /// The connector of the screen it names.
    pub fn output(&self) -> Option<&str> {
        match self {
            Place::Preferred(name) | Place::Remembered(name) | Place::Spare(name) => Some(name),
            Place::Nowhere => None,
        }
    }
}

/// Decides where the stage goes among the screens `connected`:
/// `preferred` from the settings, `remembered` for this setup, and the
/// screen the dashboard is on, to keep off it.
pub fn choose(
    connected: &[Output],
    preferred: Option<&str>,
    remembered: Option<&str>,
    dashboard: Option<&str>,
) -> Place {
    let find = |wanted: Option<&str>| {
        wanted.and_then(|wanted| connected.iter().find(|output| output.is(wanted)))
    };
    if let Some(output) = find(preferred) {
        return Place::Preferred(output.name.clone());
    }
    if connected.len() < 2 {
        return Place::Nowhere;
    }
    if let Some(output) = find(remembered) {
        return Place::Remembered(output.name.clone());
    }
    match connected
        .iter()
        .find(|output| Some(output.name.as_str()) != dashboard)
    {
        Some(output) => Place::Spare(output.name.clone()),
        None => Place::Nowhere,
    }
}

/// Where the stage was last put by hand, for each set of screens: a file
/// of `setup<TAB>screen` lines beside the saved plans.
#[derive(Debug, Clone)]
pub struct Memory {
    file: PathBuf,
}

impl Memory {
    pub fn new(data: &Path) -> Memory {
        Memory::named(data, "screens")
    }

    /// Something else kept by setup, in the file `name`: one thing for
    /// each set of screens.
    pub fn named(data: &Path, name: &str) -> Memory {
        Memory {
            file: data.join(name),
        }
    }

    fn lines(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(&self.file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(setup, screen)| (setup.to_string(), screen.to_string()))
            .collect()
    }

    /// The screen the stage was last put on with `setup` connected.
    pub fn of(&self, setup: &str) -> Option<String> {
        self.lines()
            .into_iter()
            .find(|(kept, _)| kept == setup)
            .map(|(_, screen)| screen)
    }

    /// Keeps `screen` as where the stage goes with `setup` connected.
    pub fn keep(&self, setup: &str, screen: &str) -> std::io::Result<()> {
        let mut lines = self.lines();
        lines.retain(|(kept, _)| kept != setup);
        lines.push((setup.to_string(), screen.to_string()));
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text: String = lines
            .iter()
            .map(|(setup, screen)| format!("{setup}\t{screen}\n"))
            .collect();
        std::fs::write(&self.file, text)
    }
}

/// Why the outputs can't be listed or chosen here.
pub const UNSUPPORTED: &str = "Choosing the output needs Hyprland for now. Elsewhere, move the stage window with your window manager.";

/// Whether the compositor is one this can talk to.
pub fn supported() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

#[derive(Deserialize)]
struct Monitor {
    id: i64,
    name: String,
    #[serde(default)]
    description: String,
    width: u32,
    height: u32,
    #[serde(rename = "activeWorkspace")]
    active_workspace: Workspace,
}

#[derive(Deserialize)]
struct Workspace {
    id: i64,
}

#[derive(Deserialize)]
struct Client {
    class: String,
    pid: u32,
    monitor: i64,
    address: String,
}

fn hyprctl<T: for<'de> Deserialize<'de>>(what: &str) -> Result<T, String> {
    let output = Command::new("hyprctl")
        .args([what, "-j"])
        .output()
        .map_err(|err| format!("hyprctl: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "hyprctl {what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|err| format!("hyprctl {what}: {err}"))
}

fn monitors() -> Result<Vec<Monitor>, String> {
    if !supported() {
        return Err(UNSUPPORTED.into());
    }
    hyprctl("monitors")
}

/// The screens connected now, in the compositor's order.
pub fn list() -> Result<Vec<Output>, String> {
    Ok(monitors()?
        .into_iter()
        .map(|m| Output {
            name: m.name,
            description: m.description,
            width: m.width,
            height: m.height,
        })
        .collect())
}

/// The output this process's stage window is on, once it's mapped.
pub fn stage_on() -> Result<Option<String>, String> {
    let monitors = monitors()?;
    let Some(stage) = stage_client()? else {
        return Ok(None);
    };
    Ok(monitors
        .into_iter()
        .find(|m| m.id == stage.monitor)
        .map(|m| m.name))
}

/// The output this process's dashboard window is on.
pub fn dashboard_on() -> Result<Option<String>, String> {
    let monitors = monitors()?;
    let pid = std::process::id();
    let clients: Vec<Client> = hyprctl("clients")?;
    let Some(dashboard) = clients
        .into_iter()
        .find(|c| c.class == crate::DASHBOARD_APP_ID && c.pid == pid)
    else {
        return Ok(None);
    };
    Ok(monitors
        .into_iter()
        .find(|m| m.id == dashboard.monitor)
        .map(|m| m.name))
}

fn stage_client() -> Result<Option<Client>, String> {
    let pid = std::process::id();
    let clients: Vec<Client> = hyprctl("clients")?;
    Ok(clients
        .into_iter()
        .find(|c| c.class == crate::STAGE_APP_ID && c.pid == pid))
}

/// Has the compositor blur what shows through the dashboard window, or
/// not. The window's own request for blur is made when it opens, and
/// Hyprland doesn't take it: there a window's blur is a property the
/// compositor keeps, set here. `Ok(false)` when the window isn't open yet.
pub fn blur_dashboard(on: bool) -> Result<bool, String> {
    let pid = std::process::id();
    let clients: Vec<Client> = hyprctl("clients")?;
    let Some(dashboard) = clients
        .into_iter()
        .find(|c| c.class == crate::DASHBOARD_APP_ID && c.pid == pid)
    else {
        return Ok(false);
    };
    let dispatch = format!(
        "hl.dsp.window.set_prop({{ window = \"address:{}\", prop = \"no_blur\", value = \"{}\" }})",
        dashboard.address,
        u8::from(!on)
    );
    let done = Command::new("hyprctl")
        .args(["dispatch", &dispatch])
        .output()
        .map_err(|err| format!("hyprctl: {err}"))?;
    let said = String::from_utf8_lossy(&done.stdout);
    if !done.status.success() || !said.trim().eq_ignore_ascii_case("ok") {
        return Err(format!("hyprctl dispatch: {}", said.trim()));
    }
    Ok(true)
}

/// Moves the stage window to `output`'s visible workspace, where it stays
/// fullscreen. `output` is a connector, or a screen's make and model.
/// Nothing to do when it's there already or not mapped yet.
pub fn put_stage_on(output: &str) -> Result<(), String> {
    let monitors = monitors()?;
    let target = monitors
        .iter()
        .find(|m| m.name == output || m.description.trim() == output.trim())
        .ok_or_else(|| format!("{output} isn't connected"))?;
    let Some(stage) = stage_client()? else {
        return Err("the stage window isn't open yet".into());
    };
    if stage.monitor == target.id {
        return Ok(());
    }
    let dispatch = format!(
        "hl.dsp.window.move({{ workspace = {}, window = \"address:{}\" }})",
        target.active_workspace.id, stage.address
    );
    let done = Command::new("hyprctl")
        .args(["dispatch", &dispatch])
        .output()
        .map_err(|err| format!("hyprctl: {err}"))?;
    let said = String::from_utf8_lossy(&done.stdout);
    if !done.status.success() || !said.trim().eq_ignore_ascii_case("ok") {
        return Err(format!("hyprctl dispatch: {}", said.trim()));
    }
    Ok(())
}

/// What the app last saw, to move the stage only when something happened:
/// never on every check, so a stage moved by hand stays where it was put.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Watch {
    /// The outputs connected at the last check.
    connected: BTreeSet<String>,
    /// The output asked for at the last check.
    wanted: Option<String>,
    /// Whether the stage window had been seen.
    stage_seen: bool,
}

impl Watch {
    /// Given what's connected now, the output asked for, and whether the
    /// stage window is mapped: whether to move the stage now. It moves when
    /// the stage first appears, when the asked-for output changes, and when
    /// it's plugged in; and only when that output is connected.
    pub fn check(
        &mut self,
        connected: impl IntoIterator<Item = String>,
        wanted: Option<&str>,
        stage_mapped: bool,
    ) -> bool {
        let connected: BTreeSet<String> = connected.into_iter().collect();
        let wanted_owned = wanted.map(str::to_string);
        let move_now = match wanted {
            Some(output) if connected.contains(output) && stage_mapped => {
                !self.stage_seen || self.wanted != wanted_owned || !self.connected.contains(output)
            }
            _ => false,
        };
        self.connected = connected;
        self.wanted = wanted_owned;
        // A stage that went away and came back has appeared again.
        self.stage_seen = stage_mapped;
        move_now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_stage_moves_when_it_appears_when_asked_and_when_plugged_in() {
        let mut watch = Watch::default();
        let laptop = names(&["eDP-1"]);
        let both = names(&["eDP-1", "HDMI-A-1"]);

        assert!(
            !watch.check(both.clone(), Some("HDMI-A-1"), false),
            "not mapped yet"
        );
        assert!(
            watch.check(both.clone(), Some("HDMI-A-1"), true),
            "it appeared"
        );
        assert!(
            !watch.check(both.clone(), Some("HDMI-A-1"), true),
            "moved by hand stays"
        );

        assert!(
            !watch.check(laptop.clone(), Some("HDMI-A-1"), true),
            "unplugged: nowhere to go"
        );
        assert!(
            watch.check(both.clone(), Some("HDMI-A-1"), true),
            "plugged back in"
        );

        assert!(
            watch.check(both.clone(), Some("eDP-1"), true),
            "asked for another"
        );
        assert!(
            !watch.check(both.clone(), None, true),
            "automatic leaves it alone"
        );
        assert!(watch.check(both, Some("eDP-1"), true), "asked again");
    }

    fn screen(name: &str, model: &str) -> Output {
        Output {
            name: name.into(),
            description: model.into(),
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn the_preferred_screen_wins_and_one_screen_has_no_stage() {
        let laptop = screen("eDP-1", "BOE 0x07C9");
        let projector = screen("HDMI-A-1", "Epson EB-X41");
        let monitor = screen("HDMI-A-1", "Dell U2415");
        let church = [laptop.clone(), projector.clone()];
        let home = [laptop.clone(), monitor.clone()];
        let alone = [laptop.clone()];

        // By make and model: the same plug at home is another screen.
        assert_eq!(
            choose(&church, Some("Epson EB-X41"), None, Some("eDP-1")),
            Place::Preferred("HDMI-A-1".into())
        );
        assert_eq!(
            choose(&home, Some("Epson EB-X41"), None, Some("eDP-1")),
            Place::Spare("HDMI-A-1".into()),
            "not the projector: only a screen the dashboard isn't on"
        );
        // An older config names the plug.
        assert_eq!(
            choose(&church, Some("HDMI-A-1"), None, Some("eDP-1")),
            Place::Preferred("HDMI-A-1".into())
        );
        // Put by hand on the laptop's own screen at home: remembered there.
        assert_eq!(
            choose(
                &home,
                Some("Epson EB-X41"),
                Some("BOE 0x07C9"),
                Some("eDP-1")
            ),
            Place::Remembered("eDP-1".into())
        );
        // Preferred beats remembered.
        assert_eq!(
            choose(&church, Some("Epson EB-X41"), Some("BOE 0x07C9"), None),
            Place::Preferred("HDMI-A-1".into())
        );
        // The laptop alone: nowhere, unless its screen is the preference.
        assert_eq!(
            choose(&alone, Some("Epson EB-X41"), None, None),
            Place::Nowhere
        );
        assert_eq!(
            choose(&alone, None, Some("BOE 0x07C9"), None),
            Place::Nowhere
        );
        assert_eq!(
            choose(&alone, Some("BOE 0x07C9"), None, None),
            Place::Preferred("eDP-1".into())
        );
    }

    #[test]
    fn a_setup_is_its_screens_in_any_order_and_remembers_one_place() {
        let laptop = screen("eDP-1", "BOE 0x07C9");
        let monitor = screen("HDMI-A-1", "Dell U2415");
        let nameless = screen("DP-2", "");
        assert_eq!(
            setup(&[monitor.clone(), laptop.clone()]),
            setup(&[laptop.clone(), monitor.clone()])
        );
        assert_eq!(nameless.id(), "DP-2", "no model: its plug");
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path());
        let home = setup(&[laptop.clone(), monitor]);
        assert_eq!(memory.of(&home), None);
        memory.keep(&home, "Dell U2415").unwrap();
        memory.keep("BOE 0x07C9 + Epson", "Epson").unwrap();
        memory.keep(&home, "BOE 0x07C9").unwrap();
        assert_eq!(memory.of(&home).as_deref(), Some("BOE 0x07C9"));
        assert_eq!(memory.of("BOE 0x07C9 + Epson").as_deref(), Some("Epson"));
    }

    #[test]
    fn an_output_reads_as_its_name_model_and_size() {
        let output = Output {
            name: "HDMI-A-1".into(),
            description: "Epson PowerLite".into(),
            width: 1920,
            height: 1080,
        };
        assert_eq!(output.label(), "HDMI-A-1 · Epson PowerLite · 1920×1080");
        let bare = Output {
            description: " ".into(),
            ..output
        };
        assert_eq!(bare.label(), "HDMI-A-1 · 1920×1080");
    }
}
