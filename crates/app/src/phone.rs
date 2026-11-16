//! The remote for a phone: the app's side of `lantern-remote`.
//!
//! While the remote is on, a [`Remote`] listens on the local network. The
//! app [publishes](App::tell_remote) what the page shows after everything
//! it does, and carries out what a phone sends ([`App::remote_command`])
//! the way it would a key: the same code runs.
//!
//! A phone gets the address, with its token, from a QR code in the
//! settings. The token is made once and kept in the data folder, so an
//! address saved to a phone's home screen keeps working.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use iced::futures::channel::mpsc::{UnboundedReceiver, unbounded};
use iced::widget::svg;
use iced::{Color, Task};
use lantern_engine::{Action, Media, Playback, Step, StepId, Timing};
use lantern_remote::{
    Bang, Colors, Command, Found, Group, Hit, Live, Place, Plan, Server, Slides, Snapshot,
    StepKind, Timed, Upcoming,
};

use crate::keys::Op;
use crate::plan_pane::{section_status, slot_detail};
use crate::search;
use crate::{App, Message, length, live_search, log};

/// How many results of one source a phone is sent: more than fit a few
/// screens of a phone are better found by typing more.
const MOST_HITS: usize = 25;

/// The remote, running.
pub struct Remote {
    server: Server,
    /// The address a phone opens, token and all.
    pub address: String,
    /// The same as a QR code.
    pub code: svg::Handle,
    /// What phones sent, until the app's subscription takes it.
    pub inbox: Arc<Inbox>,
    token: String,
    /// This machine's name in [`address`](Remote::address).
    host: String,
}

/// The commands phones sent, waiting for the app.
pub struct Inbox(Mutex<Option<UnboundedReceiver<Command>>>);

impl Inbox {
    /// The commands, once: the subscription that asks first gets them.
    pub fn take(&self) -> Option<UnboundedReceiver<Command>> {
        self.0.lock().ok()?.take()
    }
}

impl Remote {
    /// Starts listening on `port`, with the token kept in `data` (made
    /// the first time).
    pub fn start(port: u16, data: &Path) -> Result<Remote, String> {
        let token = token(&data.join("remote-token"));
        let (sent, received) = unbounded();
        let server = Server::start(port, token.clone(), move |command| {
            let _ = sent.unbounded_send(command);
        })
        .map_err(|err| format!("port {port}: {err}"))?;
        let mut remote = Remote {
            server,
            address: String::new(),
            code: svg::Handle::from_memory(Vec::new()),
            inbox: Arc::new(Inbox(Mutex::new(Some(received)))),
            token,
            host: String::new(),
        };
        remote.follow_network()?;
        Ok(remote)
    }

    /// Writes the address and its code again if this machine's address
    /// on the network changed: it joined another Wi-Fi, or a hotspot.
    /// The server listens on every address, so only what the phone is
    /// told changes.
    pub fn follow_network(&mut self) -> Result<(), String> {
        let host = lan_address().map_or_else(|| "localhost".to_string(), |ip| ip.to_string());
        if host == self.host {
            return Ok(());
        }
        let port = self.server.port();
        let address = format!("http://{host}:{port}/?t={}", self.token);
        self.code = svg::Handle::from_memory(qr_svg(&address)?.into_bytes());
        self.address = address;
        log(format!("remote: listening at http://{host}:{port}"));
        self.host = host;
        Ok(())
    }

    /// Whether a phone is showing the picture of the stage.
    pub fn wants_picture(&self) -> bool {
        self.server.picture_wanted()
    }

    /// Gives the phones a picture of the stage.
    pub fn set_picture(&self, jpeg: Vec<u8>) {
        self.server.set_picture(jpeg);
    }

    /// How many other machines have reached the remote since it started.
    pub fn reached(&self) -> usize {
        self.server.reached()
    }

    /// Whether a phone has the page open.
    pub fn watched(&self) -> bool {
        self.server.watching() > 0
    }
}

/// The token in `file`, or a new one written there. A file that can't be
/// written still gives a token; it just won't outlast the app.
fn token(file: &Path) -> String {
    if let Ok(kept) = std::fs::read_to_string(file)
        && kept.trim().len() >= 16
        && kept.trim().chars().all(|c| c.is_ascii_alphanumeric())
    {
        return kept.trim().to_string();
    }
    use std::hash::{BuildHasher, Hasher, RandomState};
    // Each `RandomState` is keyed from the system's randomness.
    let random = || RandomState::new().build_hasher().finish();
    let token = format!("{:016x}{:016x}", random(), random());
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(file, &token));
    if let Err(err) = saved {
        log(format!("remote: {}: {err}", file.display()));
    }
    token
}

/// This machine's address on the local network: the one it would reach
/// other machines from. Nothing is sent; a UDP socket is only pointed
/// somewhere to see which address the system picks for it.
fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // An address reserved for documentation: never a real machine.
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// `text` as a QR code: black squares on white, with the quiet border a
/// camera needs, as an SVG.
fn qr_svg(text: &str) -> Result<String, String> {
    const BORDER: usize = 4;
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|err| err.to_string())?;
    let width = code.width();
    let side = width + 2 * BORDER;
    let mut squares = String::new();
    for (at, colour) in code.to_colors().into_iter().enumerate() {
        if colour == qrcode::Color::Dark {
            let (x, y) = (at % width + BORDER, at / width + BORDER);
            squares.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" shape-rendering=\"crispEdges\">\
         <rect width=\"{side}\" height=\"{side}\" fill=\"#ffffff\"/><path d=\"{squares}\" fill=\"#000000\"/></svg>"
    ))
}

/// The widest the picture of the stage is sent: enough for a phone held
/// upright, and a few tens of kilobytes a second.
const PICTURE_WIDTH: u32 = 640;

/// A screenshot of the stage, `rgba` of `width` by `height`, as a small
/// JPEG. Off the main thread: scaling and encoding take a moment.
pub fn picture(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let shot = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or("the screenshot is the wrong size")?;
    let small = if width > PICTURE_WIDTH {
        let scaled = (height as u64 * PICTURE_WIDTH as u64 / width as u64).max(1) as u32;
        image::imageops::thumbnail(&shot, PICTURE_WIDTH, scaled)
    } else {
        shot
    };
    let rgb = image::DynamicImage::ImageRgba8(small).into_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 70)
        .encode_image(&rgb)
        .map_err(|err| err.to_string())?;
    Ok(jpeg)
}

/// A step's id as the page knows it.
fn number(id: StepId) -> u64 {
    serde_json::to_value(id)
        .ok()
        .and_then(|id| id.as_u64())
        .unwrap_or(u64::MAX)
}

fn hex(colour: Color) -> String {
    let [r, g, b, _] = colour.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

impl App {
    /// Starts or stops the remote as the settings say: the switch as it
    /// stands while they're open, else as saved. A remote that couldn't
    /// start says why, and isn't tried again until the switch moves.
    pub(crate) fn sync_remote(&mut self) {
        let wanted = match &self.settings {
            Some(draft) => draft.remote,
            None => self.config.remote,
        };
        if !wanted {
            if self.remote.take().is_some() {
                log("remote: stopped");
            }
            self.remote_failed = None;
            return;
        }
        if let Some(remote) = &mut self.remote {
            // While the settings show the address, it is the one this
            // machine has now.
            if self.settings.is_some()
                && let Err(err) = remote.follow_network()
            {
                log(format!("remote: {err}"));
            }
            return;
        }
        if self.remote_failed.is_some() {
            return;
        }
        let port = self
            .config
            .remote_port
            .unwrap_or(crate::config::REMOTE_PORT);
        let data = crate::config::xdg_dir(crate::config::env, "XDG_DATA_HOME", ".local/share");
        match Remote::start(port, &data) {
            Ok(remote) => self.remote = Some(remote),
            Err(err) => {
                log(format!("remote: {err}"));
                self.remote_failed = Some(err);
            }
        }
    }

    /// Shows the phones how things stand, when any is looking.
    pub(crate) fn tell_remote(&self) {
        if let Some(remote) = self.remote.as_ref().filter(|remote| remote.watched()) {
            remote.server.publish(&self.snapshot());
        }
    }

    /// Carries out what a phone asked for.
    pub(crate) fn remote_command(&mut self, command: Command) -> Task<Message> {
        if command != Command::Hello {
            log(format!("remote: {command:?}"));
        }
        match command {
            // Its snapshot goes out with every update, this one included.
            Command::Hello => Task::none(),
            Command::Next => self.op(Op::Next),
            Command::Prev => self.op(Op::Prev),
            Command::PlayPause => self.op(Op::PlayPause),
            Command::Blackout => self.op(Op::Blackout),
            Command::Clear => self.op(Op::Clear),
            Command::Resume => self.update(Message::Resume),
            Command::Attachment { n } => self.update(Message::Attachment(n)),
            Command::Seek { seconds } if seconds.is_finite() && seconds >= 0.0 => {
                self.engine.0.seek(Duration::from_secs_f64(seconds));
                Task::none()
            }
            Command::Seek { .. } => Task::none(),
            Command::Search { text, enter } => {
                let engine = self.engine.0.clone();
                // The same text again, asked for outright: the search a
                // bang like `!yt` was waiting to be asked for.
                if enter && text == self.phone_asked {
                    return match self.phone_search.start_on_enter(&engine) {
                        Some(stream) => live_search(stream),
                        None => Task::none(),
                    };
                }
                self.phone_search.clear(&engine);
                let edit = self.phone_search.edit(text.clone(), &engine);
                self.phone_asked = text;
                // The phone waited out the typing itself: no pause here.
                let stream = match edit {
                    Some(edit) => self.phone_search.start_live(edit, &engine),
                    None if enter => self.phone_search.start_on_enter(&engine),
                    None => None,
                };
                stream.map_or_else(Task::none, live_search)
            }
            Command::Show { id } => {
                let Some(resource) = self.phone_found(&id) else {
                    return Task::none();
                };
                self.engine.0.show_now(resource);
                self.stepping = true;
                self.preload()
            }
            Command::Add { id } => {
                let Some(resource) = self.phone_found(&id) else {
                    return Task::none();
                };
                // After the step the service is at, or at the plan's end.
                let after = self.engine.0.current_step().map(|step| step.id);
                self.remember(None);
                self.engine.0.add_resource(after, &resource);
                self.preload()
            }
            Command::Go { step } => {
                let plan = self.engine.0.plan();
                let Some(step) = plan.steps().iter().find(|s| number(s.id) == step) else {
                    return Task::none();
                };
                // As taking a step lined up on the dashboard does.
                self.selected = None;
                if let Err(err) = self.engine.0.go_to(step.id) {
                    log(format!("remote: {err}"));
                }
                self.engine.0.clear_preview();
                Task::batch([self.preload(), self.arrive()])
            }
        }
    }

    /// The result a phone named, among what its search found.
    fn phone_found(&self, id: &str) -> Option<Arc<lantern_engine::Resource>> {
        self.phone_search
            .groups()
            .iter()
            .flat_map(|group| &group.hits)
            .find(|hit| hit.id.to_string() == id)
            .cloned()
    }

    /// The phone's search and what it found so far.
    fn found(&self) -> Option<Found> {
        if self.phone_asked.trim().is_empty() {
            return None;
        }
        let groups = self.phone_search.groups().iter().map(|group| Group {
            name: group.name.clone(),
            error: group.error.clone(),
            hits: group
                .hits
                .iter()
                .take(MOST_HITS)
                .map(|hit| Hit {
                    id: hit.id.to_string(),
                    title: hit.title.clone(),
                    facts: hit.subtitle.clone().unwrap_or_default(),
                })
                .collect(),
        });
        Some(Found {
            text: self.phone_asked.clone(),
            busy: matches!(
                self.phone_search.live,
                search::Live::Waiting(_) | search::Live::Running(_)
            ),
            waiting: self.phone_search.live == search::Live::OnEnter,
            groups: groups.collect(),
        })
    }

    /// Everything the page shows, as it stands now.
    pub(crate) fn snapshot(&self) -> Snapshot {
        let p = self.palette;
        let plan = self.engine.0.plan();
        let timing = self.timing();
        let scene = self.stage.program.as_ref();
        let live = scene.map(|scene| {
            // Under a sound, the slides are the picture's.
            let pictured = self.stage.picture().filter(|_| scene.sounds_only());
            let under = pictured.and_then(|picture| match &picture.media {
                Media::Frames {
                    frame,
                    count: Some(count),
                } if count.get() > 1 => Some(Slides {
                    at: frame + 1,
                    of: Some(count.get()),
                }),
                _ => None,
            });
            let (timed, slides, failed) = match &scene.media {
                Media::Timed(timed) => (
                    Some(Timed {
                        playing: timed.playback == Playback::Playing,
                        ended: timed.playback == Playback::Ended,
                        position: timed.position.as_secs(),
                        length: timed.duration.map(|length| length.as_secs()),
                    }),
                    under,
                    None,
                ),
                Media::Frames { frame, count } => {
                    let of = count.map(|count| count.get());
                    let several = of.is_none_or(|of| of > 1) && (*frame > 0 || of.is_some());
                    (None, several.then_some(Slides { at: frame + 1, of }), None)
                }
                Media::Failed { reason } => (None, None, Some(reason.clone())),
            };
            let on_slide = self
                .attached
                .as_ref()
                .filter(|(deck, _)| *deck == scene.resource.id)
                .and_then(|(_, slides)| slides.get(scene.frame()))
                .map_or(&[][..], Vec::as_slice);
            Live {
                title: scene.resource.title.clone(),
                facts: scene.resource.subtitle.clone().unwrap_or_default(),
                timed,
                slides,
                failed,
                attached: on_slide
                    .iter()
                    .enumerate()
                    .map(|(n, resource)| {
                        crate::dashboard::attachment_name(resource, n, on_slide.len()).1
                    })
                    .collect(),
                back: self
                    .left
                    .as_ref()
                    .filter(|_| self.left_for_program())
                    .map(|left| left.frame + 1),
            }
        });
        let next = self.engine.0.up_next().map(|step| Upcoming {
            title: match &step.action {
                Action::Show { resource } if step.label.is_empty() => resource.title.clone(),
                _ => step.label.clone(),
            },
        });
        Snapshot {
            colors: Colors {
                ground: hex(p.surface),
                card: hex(p.surface_container),
                raised: hex(p.surface_container_highest),
                text: hex(p.on_surface),
                dim: hex(p.on_surface_variant),
                accent: hex(p.primary),
                on_accent: hex(p.on_primary),
                live: hex(p.tertiary),
                danger: hex(p.error),
            },
            plan: (!plan.is_empty() || !plan.title.is_empty()).then(|| Plan {
                name: plan.title.clone(),
                steps: self.remote_steps(&plan),
            }),
            live,
            next,
            blackout: self.stage.blackout,
            off_plan: scene.is_some() && self.off_plan().is_some(),
            notes: self.standing(&timing),
            bangs: self
                .engine
                .0
                .searcher()
                .bangs()
                .into_iter()
                .map(|bang| Bang {
                    name: bang.sources.join(", "),
                    bang: bang.bang,
                })
                .collect(),
            search: self.found(),
        }
    }

    /// The plan's rows as the page shows them.
    fn remote_steps(&self, plan: &lantern_engine::Plan) -> Vec<lantern_remote::Step> {
        let current = self.engine.0.current_step().map(|step| step.id);
        let at = current.and_then(|id| plan.position(id));
        let issues = self.issues(plan);
        plan.steps()
            .iter()
            .enumerate()
            .map(|(index, step)| lantern_remote::Step {
                id: number(step.id),
                label: step.label.clone(),
                kind: match step.action {
                    Action::Section { .. } => StepKind::Section,
                    Action::Live { .. } => StepKind::Live,
                    Action::Blank => StepKind::Blank,
                    Action::Show { .. } | Action::Note => StepKind::Show,
                },
                place: match at {
                    Some(at) if index == at => Place::Current,
                    Some(at) if index < at => Place::Done,
                    _ => Place::Ahead,
                },
                detail: self.remote_detail(step),
                problem: issues
                    .iter()
                    .find(|issue| issue.step == step.id)
                    .map(|issue| issue.what.clone()),
            })
            .collect()
    }

    /// The few words at the end of a row: what kind of step it is, else
    /// how long it takes.
    fn remote_detail(&self, step: &Step) -> String {
        match &step.action {
            Action::Live { hint, shown } => slot_detail(hint.as_ref(), shown.len()),
            Action::Blank => "black".to_string(),
            Action::Note => "note".to_string(),
            Action::Section { .. } => String::new(),
            Action::Show { .. } => self.step_length(step),
        }
    }

    /// How the service stands, a few words each: the section under way
    /// against its budget, then how far behind or ahead the whole is, or
    /// when it ends.
    fn standing(&self, timing: &Timing) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(section) = timing.running() {
            notes.push(section_status(section).0);
        }
        let ends_in = |end: SystemTime| end.duration_since(SystemTime::now()).unwrap_or_default();
        match (self.drift(timing), timing.end) {
            (Some(drift), _) => notes.push(drift),
            (None, Some(end)) if timing.running().is_some() => notes.push(format!(
                "ends in {}",
                length::rough(ends_in(end)).trim_start_matches('~')
            )),
            _ => {}
        }
        notes
    }

    /// How far the service is behind or ahead of its schedule, for a plan
    /// with a start time that is under way. The strip says the same.
    fn drift(&self, timing: &Timing) -> Option<String> {
        let (starts, scheduled) = self.engine.0.edit_plan(|plan| {
            let total = plan.starts.map(|_| {
                lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step)).total
            });
            (plan.starts, total.filter(|_| !plan.is_empty()))
        });
        let ends_in = timing
            .end?
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        let behind =
            lantern_engine::timing::behind(starts?, scheduled?, length::now_of_day(), ends_in);
        Some(length::drift(behind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deeper/remote-token");
        let first = token(&file);
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(token(&file), first, "read back, not made again");
        // What isn't a token is replaced.
        std::fs::write(&file, "short").unwrap();
        let second = token(&file);
        assert_ne!(second, first);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), second);
    }

    #[test]
    fn an_address_becomes_a_code_with_a_quiet_border() {
        let svg = qr_svg("http://192.168.1.20:7373/?t=0123456789abcdef0123456789abcdef").unwrap();
        let side: usize = svg
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .unwrap()
            .parse()
            .unwrap();
        assert!(side >= 21 + 8, "{side}");
        // No square in the border: the first is at least four in.
        let first = svg.split("d=\"M").nth(1).unwrap();
        let x: usize = first.split(' ').next().unwrap().parse().unwrap();
        assert!((4..side - 4).contains(&x), "{x}");
        assert!(svg.contains("fill=\"#ffffff\"") && svg.contains("fill=\"#000000\""));
    }

    #[test]
    fn a_screenshot_becomes_a_small_jpeg() {
        let (width, height) = (1920, 1080);
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|at| [(at % 255) as u8, 80, 160, 255])
            .collect();
        let jpeg = picture(&rgba, width, height).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]), "a JPEG");
        let read = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((read.width(), read.height()), (640, 360));
        // One already small is left its size; a wrong size is an error.
        let small = picture(&rgba[..320 * 180 * 4], 320, 180).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().width(), 320);
        assert!(picture(&rgba[..100], 320, 180).is_err());
    }

    #[test]
    fn a_colour_is_written_as_the_page_reads_it() {
        assert_eq!(hex(Color::from_rgb8(0x1a, 0x11, 0x13)), "#1a1113");
        assert_eq!(hex(Color::WHITE), "#ffffff");
    }
}
