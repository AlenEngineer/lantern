//! Which screen (output) the stage is on. iced can't list monitors or
//! fullscreen a window on a chosen one, and on Wayland only the compositor
//! places windows, so this asks the compositor. Only Hyprland so far:
//! `hyprctl` lists the outputs and moves the stage window (found by its app
//! id and this process, so another Lantern is never touched).

use std::collections::BTreeSet;
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
/// fullscreen. Nothing to do when it's there already or not mapped yet.
pub fn put_stage_on(output: &str) -> Result<(), String> {
    let monitors = monitors()?;
    let target = monitors
        .iter()
        .find(|m| m.name == output)
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
        self.stage_seen |= stage_mapped;
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
