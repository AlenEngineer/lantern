//! The remote for a phone: the app's side of `lantern-remote`.
//!
//! While the remote is on, a [`Remote`] listens on the local network. The
//! app [publishes](App::tell_remote) what the page shows after everything
//! it does, and carries out what a phone sends ([`App::remote_command`])
//! the way it would a key: the same code runs.
//!
//! A phone gets the address, with its token, from a QR code in the
//! settings. The token is made once and kept in the data folder, so an
//! address saved to a phone's home screen keeps working. When this machine
//! is on Tailscale, a second code gives its Tailscale address, for a phone
//! that isn't on the same Wi-Fi.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

use iced::futures::channel::mpsc::{UnboundedReceiver, unbounded};
use iced::widget::image::Handle;
use iced::widget::svg;
use iced::{Color, Task};
use lantern_engine::{Action, Kind, Media, Playback, Resource, ResourceId, Step, StepId, Timing};
use lantern_remote::{
    Badge, Bang, BibleBook, BibleBooks, Colors, Command, Found, Group, Hit, HitKind, Listed, Live,
    Note, Place, Plan, Server, Shown, Slides, Snapshot, Source, SourceKind, Starts, StepKind,
    Timed, Tone, Upcoming,
};

use crate::before::{self, SoundChange};
use crate::keys::Op;
use crate::mode::Mode;
use crate::plan_pane::{State, Under, section_status, slot_detail, soft_words, what_of};
use crate::ready::Level;
use crate::search;
use crate::{App, Message, in_thread, length, library, live_search, log};

/// How many results of one source a phone is sent: more than fit a few
/// screens of a phone are better found by typing more.
const MOST_HITS: usize = 25;

/// How many results' pictures a phone is given at once: the first a
/// search or a listed folder shows. Each is a few kilobytes.
const MOST_THUMBS: usize = 60;

/// How often this machine's Tailscale address is looked for again while
/// the settings show it.
const LOOK_FOR_TAILSCALE: Duration = Duration::from_secs(10);

/// How long the settings wait for `tailscale ip` before going on without
/// its answer; a later look picks the answer up.
const WAIT_FOR_TAILSCALE: Duration = Duration::from_millis(150);

/// The longest `tailscale ip` may take before it is stopped.
const TAILSCALE_TIMEOUT: Duration = Duration::from_secs(2);

/// The remote, running.
pub struct Remote {
    server: Server,
    /// How a phone on the same Wi-Fi opens the page.
    pub lan: Reach,
    /// How a phone on Tailscale opens it, when this machine is on it.
    pub tailscale: Option<Reach>,
    /// What phones sent, until the app's subscription takes it.
    pub inbox: Arc<Inbox>,
    token: String,
    looking: Tailscale,
    /// The results' pictures the server has, as JPEGs: made once each,
    /// and let go once the phone no longer lists it.
    thumbs: Mutex<HashMap<ResourceId, Arc<Vec<u8>>>>,
}

/// One way a phone opens the page.
pub struct Reach {
    /// The address, token and all.
    pub address: String,
    /// The same as a QR code.
    pub code: svg::Handle,
    /// This machine's name in the address.
    host: String,
}

impl Reach {
    fn new(host: String, port: u16, token: &str) -> Result<Reach, String> {
        let address = format!("http://{host}:{port}/?t={token}");
        Ok(Reach {
            code: svg::Handle::from_memory(qr_svg(&address)?.into_bytes()),
            address,
            host,
        })
    }
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
            lan: Reach {
                address: String::new(),
                code: svg::Handle::from_memory(Vec::new()),
                host: String::new(),
            },
            tailscale: None,
            inbox: Arc::new(Inbox(Mutex::new(Some(received)))),
            token,
            looking: Tailscale::default(),
            thumbs: Mutex::default(),
        };
        remote.follow_network()?;
        Ok(remote)
    }

    /// Writes the addresses and their codes again if this machine's
    /// addresses changed: it joined another Wi-Fi or a hotspot, or
    /// Tailscale came on or went off. The server listens on every
    /// address, so only what the phone is told changes.
    pub fn follow_network(&mut self) -> Result<(), String> {
        let port = self.server.port();
        let lan = lan_address().map_or_else(|| "localhost".to_string(), host);
        if lan != self.lan.host {
            log(format!("remote: listening at http://{lan}:{port}"));
            self.lan = Reach::new(lan, port, &self.token)?;
        }
        // With Tailscale carrying all its traffic, this machine's address
        // on the network is its Tailscale one: one code says it all.
        let tailscale = self
            .looking
            .address()
            .map(host)
            .filter(|tailscale| *tailscale != self.lan.host);
        if tailscale.as_ref() != self.tailscale.as_ref().map(|reach| &reach.host) {
            self.tailscale = match tailscale {
                Some(tailscale) => {
                    log(format!(
                        "remote: over Tailscale at http://{tailscale}:{port}"
                    ));
                    Some(Reach::new(tailscale, port, &self.token)?)
                }
                None => None,
            };
        }
        Ok(())
    }

    /// Whether the server has the picture of the result `id`.
    fn has_thumb(&self, id: &ResourceId) -> bool {
        self.thumbs
            .lock()
            .is_ok_and(|thumbs| thumbs.contains_key(id))
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

/// `ip` as an address's host: an IPv6 one in brackets.
fn host(ip: IpAddr) -> String {
    let at = SocketAddr::new(ip, 0).to_string();
    at.strip_suffix(":0").unwrap_or(&at).to_string()
}

/// This machine's Tailscale address, looked for at most every
/// [`LOOK_FOR_TAILSCALE`].
#[derive(Default)]
struct Tailscale {
    looked: Option<Instant>,
    found: Arc<Mutex<Option<IpAddr>>>,
}

impl Tailscale {
    /// The address on a network interface of this machine's, else the one
    /// `tailscale ip` gives: Tailscale run without a network interface of
    /// its own (`--tun=userspace-networking`) puts its address on none.
    /// Windows always has the interface, so it is never asked there.
    fn address(&mut self) -> Option<IpAddr> {
        if self
            .looked
            .is_none_or(|looked| looked.elapsed() >= LOOK_FOR_TAILSCALE)
        {
            self.looked = Some(Instant::now());
            let interfaces = if_addrs::get_if_addrs().unwrap_or_default();
            match tailscale_address(interfaces.iter().map(if_addrs::Interface::ip)) {
                Some(ip) => *self.found.lock().ok()? = Some(ip),
                None if cfg!(windows) => *self.found.lock().ok()? = None,
                None => {
                    let found = Arc::clone(&self.found);
                    let (done, asked) = mpsc::channel();
                    std::thread::spawn(move || {
                        let said = ask_tailscale();
                        if let Ok(mut found) = found.lock() {
                            *found = said;
                        }
                        let _ = done.send(());
                    });
                    let _ = asked.recv_timeout(WAIT_FOR_TAILSCALE);
                }
            }
        }
        *self.found.lock().ok()?
    }
}

/// The Tailscale address `tailscale ip` gives, if Tailscale is installed
/// and up.
fn ask_tailscale() -> Option<IpAddr> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new("tailscale")
        .arg("ip")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if started.elapsed() > TAILSCALE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut said = String::new();
    child.stdout.take()?.read_to_string(&mut said).ok()?;
    status
        .success()
        .then(|| tailscale_address(said.lines().filter_map(|line| line.trim().parse().ok())))
        .flatten()
}

/// The first of `addresses` Tailscale gives out: an IPv4 one in
/// 100.64.0.0/10, else an IPv6 one in fd7a:115c:a1e0::/48. A phone on
/// mobile data is more likely to reach the IPv4 one.
fn tailscale_address(addresses: impl IntoIterator<Item = IpAddr>) -> Option<IpAddr> {
    let mut v6 = None;
    for ip in addresses {
        match ip {
            IpAddr::V4(v4) if v4.octets()[0] == 100 && v4.octets()[1] & 0xc0 == 64 => {
                return Some(ip);
            }
            IpAddr::V6(v6_ip)
                if v6.is_none() && v6_ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0] =>
            {
                v6 = Some(ip);
            }
            _ => {}
        }
    }
    v6
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
            self.give_thumbs(remote);
            remote.server.publish(&self.snapshot());
        }
    }

    /// Gives the server the pictures of the results the phone lists, as
    /// far as they are made: each made into a JPEG once, while listed.
    fn give_thumbs(&self, remote: &Remote) {
        let Ok(mut given) = remote.thumbs.lock() else {
            return;
        };
        let mut listed = HashMap::new();
        for resource in self.phone_results().take(MOST_THUMBS) {
            let jpeg = match given.get(&resource.id) {
                Some(jpeg) => Some(jpeg.clone()),
                None => self
                    .thumbs
                    .whole(resource)
                    .and_then(|(handle, _)| jpeg_of(&handle))
                    .map(Arc::new),
            };
            if let Some(jpeg) = jpeg {
                listed.insert(resource.id.clone(), jpeg);
            }
        }
        if listed.len() == given.len() && listed.keys().all(|id| given.contains_key(id)) {
            return;
        }
        remote.server.set_thumbs(
            listed
                .iter()
                .map(|(id, jpeg)| (id.to_string(), jpeg.clone()))
                .collect(),
        );
        *given = listed;
    }

    /// The results the phone lists, in order: its search's, then the
    /// folder it asked to list.
    fn phone_results(&self) -> impl Iterator<Item = &Arc<Resource>> {
        let found = self
            .phone_search
            .groups()
            .iter()
            .filter(|_| !self.phone_asked.trim().is_empty())
            .flat_map(|group| group.hits.iter().take(MOST_HITS));
        found.chain(self.phone_listed.iter().flat_map(|(_, files)| files))
    }

    /// Carries out what a phone asked for.
    pub(crate) fn remote_command(&mut self, command: Command) -> Task<Message> {
        if command != Command::Hello {
            log(format!("remote: {command:?}"));
        }
        // What moves the stage from the phone is said on this screen, so
        // whoever sits at it knows it wasn't them.
        if !matches!(
            command,
            Command::Hello
                | Command::Search { .. }
                | Command::Browse { .. }
                | Command::LineUp { .. }
                | Command::Seek { .. }
                | Command::Volume { .. }
                | Command::Mute { .. }
        ) {
            self.motion.phone_acted();
        }
        match command {
            // Its snapshot goes out with every update, this one included;
            // the sound's when it has been asked.
            Command::Hello => self.update(Message::HearSound),
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
            Command::Volume { level } if level.is_finite() => {
                self.change_sound(SoundChange::Volume(level))
            }
            Command::Volume { .. } => Task::none(),
            Command::Mute { on } => self.change_sound(SoundChange::Muted(on)),
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
            Command::LineUp { id } => {
                let Some(resource) = self.phone_found(&id) else {
                    return Task::none();
                };
                // As lining one up from the dashboard's shelf does.
                self.selected = None;
                self.editing = None;
                self.search.unpick();
                self.engine.0.preview(resource);
                self.preload()
            }
            Command::Again { n } => {
                let Some(entry) = self.engine.0.log().into_iter().nth(n) else {
                    return Task::none();
                };
                self.engine.0.show_now(Arc::new(entry.resource));
                self.stepping = true;
                self.preload()
            }
            Command::Browse { folder } => {
                self.phone_listed = folder.and_then(|n| {
                    let entry = self.config.folders.get(n)?;
                    let query =
                        lantern_engine::Query::new("").with_source(library::folder_source(entry));
                    let mut files: Vec<_> = self
                        .engine
                        .0
                        .search_index(&query)
                        .into_iter()
                        .map(|hit| hit.resource)
                        .filter(|file| {
                            file.location
                                .as_path()
                                .is_some_and(|path| path.starts_with(&entry.path))
                        })
                        .collect();
                    files.sort_by(|a, b| lantern_engine::natural(&a.title, &b.title));
                    Some((n, files))
                });
                Task::none()
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

    /// Changes the computer's sound, and shows the phones it changed
    /// without waiting for the next look.
    fn change_sound(&mut self, change: SoundChange) -> Task<Message> {
        if let Some(sound) = &mut self.phone_sound {
            match change {
                SoundChange::Volume(level) => {
                    sound.volume = level.clamp(0.0, 1.0);
                    sound.muted = false;
                }
                SoundChange::Muted(on) => sound.muted = on,
            }
        }
        Task::perform(
            in_thread(move || before::change_sound(change)),
            Message::Heard,
        )
    }

    /// The result a phone named, among what its search found.
    fn phone_found(&self, id: &str) -> Option<Arc<lantern_engine::Resource>> {
        let listed = self.phone_listed.iter().flat_map(|(_, files)| files);
        self.phone_search
            .groups()
            .iter()
            .flat_map(|group| &group.hits)
            .chain(listed)
            .find(|hit| hit.id.to_string() == id)
            .cloned()
    }

    /// What Live on the phone starts from: a tile for each source.
    fn phone_sources(&self) -> Vec<Source> {
        let registered: Vec<String> = self
            .engine
            .0
            .searcher()
            .sources()
            .iter()
            .map(|source| source.id().to_string())
            .collect();
        let has = |id: &str| registered.iter().any(|source| source == id);
        let tile = |kind, name: &str, bang: &str, hint: String| Source {
            kind,
            name: name.into(),
            bang: bang.into(),
            hint,
            folder: None,
        };
        let mut sources = Vec::new();
        let bibles: Vec<String> = self
            .library
            .0
            .outlines()
            .into_iter()
            .map(|outline| outline.name)
            .collect();
        if !bibles.is_empty() {
            let hint = format!("{} · by book", bibles.join(", "));
            sources.push(tile(SourceKind::Bible, "Bible", "b", hint));
        }
        for (n, entry) in self.config.folders.iter().enumerate() {
            let bang = entry.bang.clone().unwrap_or_else(|| "f".into());
            sources.push(Source {
                folder: Some(n),
                ..tile(
                    SourceKind::Folder,
                    &library::folder_name(entry),
                    &bang,
                    format!("your folder · !{bang}"),
                )
            });
        }
        if has("youtube") {
            sources.push(tile(
                SourceKind::Youtube,
                "YouTube",
                "yt",
                "search videos".into(),
            ));
        }
        if has("text") {
            sources.push(tile(SourceKind::Text, "Text", "x", "type a slide".into()));
        }
        if has("clock") {
            sources.push(tile(
                SourceKind::Time,
                "Time",
                "t",
                "clock, countdowns".into(),
            ));
        }
        if has("images") {
            sources.push(tile(
                SourceKind::Images,
                "Images",
                "gi",
                "web pictures".into(),
            ));
        }
        sources
    }

    /// The first Bible's books, for the phone's picker.
    fn phone_bible(&self) -> Option<BibleBooks> {
        let outline = self.library.0.outlines().into_iter().next()?;
        Some(BibleBooks {
            name: outline.name,
            books: outline
                .books
                .into_iter()
                .map(|book| BibleBook {
                    name: book.name,
                    short: book.short,
                    verses: book.verses,
                })
                .collect(),
        })
    }

    /// What was shown since Live began, for showing again.
    fn phone_shown(&self) -> Vec<Shown> {
        let on = self.stage.program.as_ref().map(|scene| &scene.resource.id);
        self.engine
            .0
            .log()
            .into_iter()
            .enumerate()
            .skip(self.live_from)
            .map(|(n, entry)| Shown {
                n,
                at: chrono::DateTime::<chrono::Local>::from(entry.at)
                    .format("%H:%M")
                    .to_string(),
                live: on == Some(&entry.resource.id),
                title: entry.resource.title,
            })
            .collect()
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
                .map(|hit| self.remote_hit(hit))
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
            detail: self.remote_detail(&step),
        });
        let in_live = self.mode == Mode::Live;
        let starts = plan.starts.map(|at| Starts {
            at: at.to_string(),
            in_seconds: at
                .since_midnight()
                .checked_sub(length::now_of_day())
                .map(|left| left.as_secs()),
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
            mode: match self.mode {
                Mode::Plan => lantern_remote::Mode::Plan,
                Mode::Run => lantern_remote::Mode::Run,
                Mode::Live => lantern_remote::Mode::Live,
            },
            starts,
            plan: (!plan.is_empty() || !plan.title.is_empty()).then(|| Plan {
                name: plan.title.clone(),
                steps: self.remote_steps(&plan, &timing),
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
            sources: if in_live {
                self.phone_sources()
            } else {
                Vec::new()
            },
            bible: in_live.then(|| self.phone_bible()).flatten(),
            shown: if in_live {
                self.phone_shown()
            } else {
                Vec::new()
            },
            listed: self.phone_listed.as_ref().map(|(folder, files)| Listed {
                folder: *folder,
                hits: files.iter().map(|file| self.remote_hit(file)).collect(),
            }),
            sound: self
                .phone_sound
                .as_ref()
                .map(|sound| lantern_remote::Sound {
                    output: sound.output.clone(),
                    volume: sound.volume,
                    muted: sound.muted,
                }),
        }
    }

    /// A result as the phone lists it: as the dashboard's row has it
    /// ([`App::result_facts`]).
    fn remote_hit(&self, resource: &Resource) -> Hit {
        let facts = self.result_facts(resource);
        let picture = self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.has_thumb(&resource.id));
        let shape = self
            .thumbs
            .pixels(&resource.id)
            .map(|pixels| pixels.width as f32 / pixels.height.max(1) as f32)
            .or_else(|| {
                picture
                    .then(|| self.thumbs.whole(resource))
                    .flatten()
                    .map(|(_, shape)| shape)
            });
        let state = facts.thumb.state;
        Hit {
            id: resource.id.to_string(),
            title: resource.title.clone(),
            facts: what_of(resource),
            text: match &resource.location {
                lantern_engine::Location::Inline(text) => text.chars().take(800).collect(),
                _ => String::new(),
            },
            kind: hit_kind(resource),
            size: facts.thumb.size,
            state: state.map(|state| Badge {
                text: state.words(),
                tone: match state {
                    State::Live => Tone::Live,
                    State::Online | State::Coming(_) => Tone::Plain,
                    State::Soft(_) => Tone::Waiting,
                },
            }),
            warning: match facts.under {
                Under::What(_) => None,
                Under::Wrong(level, text) => Some(Badge {
                    text,
                    tone: match level {
                        Level::Waiting => Tone::Waiting,
                        Level::Problem => Tone::Problem,
                    },
                }),
                Under::Soft(screen) => Some(Badge {
                    text: soft_words(screen),
                    tone: Tone::Waiting,
                }),
            },
            progress: match state {
                Some(State::Coming(part)) => Some(part),
                _ => None,
            },
            shape,
            picture,
        }
    }

    /// The plan's rows as the page shows them.
    fn remote_steps(
        &self,
        plan: &lantern_engine::Plan,
        timing: &Timing,
    ) -> Vec<lantern_remote::Step> {
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
                pace: remote_pace(
                    timing
                        .sections
                        .iter()
                        .find(|section| section.id == step.id)
                        .and_then(|section| section.pace()),
                ),
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
            Action::Section { budget, .. } => budget.map(length::rough).unwrap_or_default(),
            Action::Show { .. } => self.step_length(step),
        }
    }

    /// How the service stands, a few words each: the section under way
    /// against its budget, then how far behind or ahead the whole is, or
    /// when it ends.
    fn standing(&self, timing: &Timing) -> Vec<Note> {
        let mut notes = Vec::new();
        if let Some(section) = timing.running() {
            notes.push(Note {
                text: section_status(section).0,
                pace: remote_pace(section.pace()),
            });
        }
        let ends_in = |end: SystemTime| end.duration_since(SystemTime::now()).unwrap_or_default();
        match (self.drift(timing), timing.end) {
            (Some(drift), _) => notes.push(Note {
                text: drift,
                pace: lantern_remote::Pace::OnTrack,
            }),
            (None, Some(end)) if timing.running().is_some() => notes.push(Note {
                text: format!(
                    "ends in {}",
                    length::rough(ends_in(end)).trim_start_matches('~')
                ),
                pace: lantern_remote::Pace::OnTrack,
            }),
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

/// A thumbnail as a JPEG, for a phone.
fn jpeg_of(handle: &Handle) -> Option<Vec<u8>> {
    let Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = handle
    else {
        return None;
    };
    picture(pixels, *width, *height).ok()
}

/// What a result is, for the page's picture when it has none.
fn hit_kind(resource: &Resource) -> HitKind {
    if lantern_source_clock::Clock::of(resource).is_some() {
        return HitKind::Time;
    }
    match &resource.kind {
        Kind::Video => HitKind::Video,
        Kind::Image => HitKind::Picture,
        Kind::Presentation => HitKind::Deck,
        Kind::Document => HitKind::Pdf,
        Kind::Audio => HitKind::Sound,
        Kind::Text if resource.meta.contains_key("verses") => HitKind::Passage,
        Kind::Text => HitKind::Text,
        Kind::Web => HitKind::Web,
        Kind::Other(_) => HitKind::Other,
    }
}

fn remote_pace(pace: Option<lantern_engine::Pace>) -> lantern_remote::Pace {
    match pace {
        None | Some(lantern_engine::Pace::OnTrack) => lantern_remote::Pace::OnTrack,
        Some(lantern_engine::Pace::Tight) => lantern_remote::Pace::Tight,
        Some(lantern_engine::Pace::Over) => lantern_remote::Pace::Over,
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
    fn a_tailscale_address_is_told_from_the_others() {
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();
        let lan = [
            ip("127.0.0.1"),
            ip("192.168.1.20"),
            ip("::1"),
            ip("fe80::1"),
        ];
        let v4 = ip("100.101.102.103");
        let v6 = ip("fd7a:115c:a1e0::1");
        assert_eq!(tailscale_address(lan.into_iter().chain([v4])), Some(v4));
        assert_eq!(tailscale_address(lan.into_iter().chain([v6])), Some(v6));
        assert_eq!(tailscale_address([v6, v4]), Some(v4), "IPv4 first");
        assert_eq!(tailscale_address(lan), None);
        assert_eq!(
            tailscale_address([
                ip("100.63.255.255"),
                ip("100.128.0.1"),
                ip("fd7a:115c:a1e1::1")
            ]),
            None,
            "next to Tailscale's ranges, not in them"
        );
        assert_eq!(
            tailscale_address([ip("100.64.0.0")]),
            Some(ip("100.64.0.0"))
        );
        let edge = ip("100.127.255.255");
        assert_eq!(tailscale_address([edge]), Some(edge));
    }

    #[test]
    fn an_ipv6_host_is_in_brackets() {
        let address = |ip: &str| {
            Reach::new(host(ip.parse().unwrap()), 7373, "abc")
                .unwrap()
                .address
        };
        assert_eq!(
            address("fd7a:115c:a1e0::1"),
            "http://[fd7a:115c:a1e0::1]:7373/?t=abc"
        );
        assert_eq!(
            address("100.101.102.103"),
            "http://100.101.102.103:7373/?t=abc"
        );
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
