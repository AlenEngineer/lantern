//! A relay between the stream player and YouTube, on this machine only.
//!
//! YouTube serves a video file read in one go at about twice the speed it
//! plays, so a jump far ahead in a stream took many seconds to fill. Read
//! in pieces of a few megabytes, the same file comes as fast as the line
//! allows; that is how `yt-dlp` downloads. GStreamer's HTTP source reads in
//! one go, so the player is given an address here instead
//! ([`through`]), and each of its requests is answered by asking YouTube
//! for one piece after another.
//!
//! It also writes the player a manifest for a video's two files
//! ([`manifest`]). Each file starts with an index of where every few
//! seconds of it begin; a player handed the files alone doesn't read it,
//! and finds a place by walking the file a stretch at a time, a minute of
//! frozen picture for a jump of an hour. The manifest says where the index
//! is, and the player that takes one jumps straight there.
//!
//! It listens on the loopback address alone, on a port the system picks,
//! and serves only the addresses it was given, each under a name of its
//! own.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use ureq::ResponseExt;
use ureq::http::{HeaderMap, StatusCode};

/// The most that is asked of YouTube at a time: under the ten megabytes
/// `yt-dlp` asks for, which YouTube doesn't slow down.
const PIECE: u64 = 8 * 1024 * 1024;

/// How much the first pieces of an answer ask for, before [`PIECE`].
///
/// A jump far ahead makes the player walk the file: it reads the few
/// kilobytes that head each stretch of the video to learn where the next
/// one starts, a hundred requests for a jump of some minutes. A first
/// piece this small has arrived whole by the time the player hangs up, so
/// the connection to YouTube is kept for the next request instead of
/// being made again.
const FIRST_PIECES: [u64; 2] = [128 * 1024, 1024 * 1024];

/// How many addresses are remembered. A stream takes two, and one that is
/// still loaded must keep its own however many were looked at since.
const REMEMBERED: usize = 512;

/// How many times in a row a piece is asked for again before giving up.
/// Any byte that arrives starts the count again.
const TRIES: usize = 3;

/// How long to wait, in milliseconds, before each new try of a first
/// piece that YouTube refused. An address `yt-dlp` has only just been
/// given is refused about half the time for its first second or two
/// (403 at once, fine two seconds later, in three fresh addresses of six;
/// once for over five seconds), and without these the stream failed and
/// the video was downloaded. They add up to under ten seconds.
const REFUSED_WAITS: &[u64] = &[300, 500, 700, 1000, 1500, 2500, 3000];

/// A local address that serves `url` in pieces, or `None` when the relay
/// can't listen; the player then reads `url` itself, slower.
pub fn through(url: &str) -> Option<String> {
    relay().map(|relay| relay.serve(url))
}

/// The app's relay, started the first time it is needed.
fn relay() -> Option<&'static Arc<Relay>> {
    static RELAY: OnceLock<Option<Arc<Relay>>> = OnceLock::new();
    let relay = RELAY.get_or_init(|| match Relay::start(PIECE, REFUSED_WAITS) {
        Ok(relay) => Some(relay),
        Err(err) => {
            crate::log(format!(
                "relay: can't listen ({err}), streams read directly"
            ));
            None
        }
    });
    relay.as_ref()
}

/// A manifest for a video kept as two files, its picture and its sound,
/// that tells the player where each file's index is. `None` when the
/// relay can't listen or a file isn't laid out as expected; the files
/// then play without one.
pub fn manifest(video: &Track, audio: &Track, seconds: f64) -> Option<String> {
    let relay = relay()?;
    match relay.manifest(video, audio, seconds) {
        Ok(address) => Some(address),
        Err(err) => {
            crate::log(format!(
                "relay: no manifest ({err}), the files play without"
            ));
            None
        }
    }
}

/// One of a video's two files, as `yt-dlp` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub url: String,
    /// As in `avc1.4D401F` or `mp4a.40.2`.
    pub codec: String,
    /// Its bitrate, in bits a second.
    pub bits: u64,
}

/// What the relay serves under a name.
#[derive(Clone)]
enum Served {
    /// A file at YouTube, at this address.
    File(String),
    /// A manifest, which is this text.
    Manifest(String),
}

struct Relay {
    at: SocketAddr,
    piece: u64,
    /// [`REFUSED_WAITS`], or shorter ones for a test.
    refused_waits: &'static [u64],
    agent: ureq::Agent,
    /// The addresses served, by the name each is served under, oldest
    /// first. An address is replaced by where YouTube redirected it, so
    /// the redirect is followed once and not for every piece.
    served: Mutex<VecDeque<(String, Served)>>,
}

impl Relay {
    /// Starts listening, on a thread of its own that lasts as long as
    /// the app.
    fn start(piece: u64, refused_waits: &'static [u64]) -> io::Result<Arc<Relay>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(15)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .build()
            .new_agent();
        let relay = Arc::new(Relay {
            at: listener.local_addr()?,
            piece,
            refused_waits,
            agent,
            served: Mutex::new(VecDeque::new()),
        });
        let serving = Arc::clone(&relay);
        std::thread::spawn(move || {
            for player in listener.incoming().flatten() {
                let relay = Arc::clone(&serving);
                std::thread::spawn(move || relay.answer(player));
            }
        });
        Ok(relay)
    }

    /// The local address `url` is served at from now on.
    fn serve(&self, url: &str) -> String {
        self.serve_as(Served::File(url.to_string()), "")
    }

    /// The local address `what` is served at from now on, its name ending
    /// in `ending`.
    fn serve_as(&self, what: Served, ending: &str) -> String {
        let mut served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        let name = format!(
            "{:016x}{:x}{ending}",
            RandomState::new().build_hasher().finish(),
            served.len()
        );
        if served.len() == REMEMBERED {
            served.pop_front();
        }
        served.push_back((name.clone(), what));
        format!("http://{}/{name}", self.at)
    }

    fn served_as(&self, name: &str) -> Option<Served> {
        let served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        let found = served.iter().find(|(served, _)| served == name);
        found.map(|(_, what)| what.clone())
    }

    /// Remembers that the file served as `name` is at `url` now.
    fn moved(&self, name: &str, url: &str) {
        let mut served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, Served::File(old))) = served.iter_mut().find(|(served, _)| served == name) {
            *old = url.to_string();
        }
    }

    fn upstream(&self, url: &str, bounded: bool) -> Upstream<'_> {
        Upstream {
            agent: &self.agent,
            url: url.to_string(),
            most: self.piece,
            asked: 0,
            bounded,
            refused_waits: self.refused_waits,
        }
    }

    /// Serves both files, reads where each keeps its index, and serves a
    /// manifest that says so. Returns the manifest's address.
    fn manifest(&self, video: &Track, audio: &Track, seconds: f64) -> Result<String, String> {
        let mut sets = String::new();
        for (kind, track) in [("video", video), ("audio", audio)] {
            let address = self.serve(&track.url);
            let name = address.rsplit('/').next().unwrap_or_default();
            // The start of the file: its header, then its index.
            let mut upstream = self.upstream(&track.url, true);
            let mut head = Vec::new();
            upstream
                .first_piece(0, Some(HEAD - 1))
                .map_err(|status| format!("YouTube answered {status}"))?
                .body
                .take()
                .ok_or("no body")?
                .read_to_end(&mut head)
                .map_err(|err| err.to_string())?;
            self.moved(name, &upstream.url);
            let layout = Layout::of(&head).ok_or_else(|| format!("the {kind} has no index"))?;
            sets.push_str(&format!(
                "<AdaptationSet contentType=\"{kind}\" mimeType=\"{kind}/mp4\" subsegmentAlignment=\"true\">\
                 <Representation id=\"{kind}\" bandwidth=\"{}\" codecs=\"{}\">\
                 <BaseURL>{address}</BaseURL>\
                 <SegmentBase indexRange=\"{}-{}\"><Initialization range=\"0-{}\"/></SegmentBase>\
                 </Representation></AdaptationSet>",
                track.bits.max(1),
                xml(&track.codec),
                layout.index.0,
                layout.index.1,
                layout.header_end,
            ));
        }
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
             mediaPresentationDuration=\"PT{seconds:.3}S\" minBufferTime=\"PT2S\" \
             profiles=\"urn:mpeg:dash:profile:isoff-on-demand:2011\">\
             <Period>{sets}</Period></MPD>"
        );
        Ok(self.serve_as(Served::Manifest(text), ".mpd"))
    }

    /// Answers one request of the player's: the file from where it asked
    /// to its end, fetched a piece at a time. The player hangs up when it
    /// jumps elsewhere, which ends this.
    fn answer(&self, mut player: TcpStream) {
        let Some(asked) = Asked::read(&player) else {
            return;
        };
        let url = match self.served_as(&asked.name) {
            Some(Served::File(url)) => url,
            Some(Served::Manifest(text)) => {
                let _ = player.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/dash+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    )
                    .as_bytes(),
                );
                return;
            }
            None => {
                let _ = refuse(&mut player, StatusCode::NOT_FOUND);
                return;
            }
        };
        let mut upstream = self.upstream(&url, asked.to.is_some());
        let mut at = asked.from;
        let mut first = match upstream.first_piece(at, asked.to) {
            Ok(first) => first,
            Err(status) => {
                crate::log(format!("relay: YouTube answered {status} at {at}"));
                let _ = refuse(&mut player, status);
                return;
            }
        };
        if upstream.url != url {
            self.moved(&asked.name, &upstream.url);
        }
        let last = asked
            .to
            .map_or(first.total - 1, |to| to.min(first.total - 1));
        let head = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
            if asked.ranged {
                "206 Partial Content"
            } else {
                "200 OK"
            },
            first.kind,
            if asked.ranged {
                format!("Content-Range: bytes {at}-{last}/{}\r\n", first.total)
            } else {
                String::new()
            },
            (last + 1).saturating_sub(at),
        );
        if player.write_all(head.as_bytes()).is_err() {
            return;
        }
        let mut body = Some(first.body.take().expect("a piece comes with its body"));
        let mut failed = 0;
        let mut buffer = vec![0; 64 * 1024];
        while at <= last {
            let mut reader = match body.take() {
                Some(reader) => reader,
                // A player that jumped elsewhere is asked nothing more
                // for: its connection to YouTube stays fit to use again.
                None if hung_up(&player) => return,
                None => match upstream.piece_from(at, Some(last)) {
                    Ok(mut piece) => piece.body.take().expect("a piece comes with its body"),
                    Err(_) if failed < TRIES => {
                        failed += 1;
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                    // Hanging up short of the length promised tells the
                    // player the stream broke.
                    Err(_) => return,
                },
            };
            loop {
                match reader.read(&mut buffer) {
                    // The piece is done, or was cut short: the next one
                    // starts where this stopped.
                    Ok(0) => break,
                    Ok(got) => {
                        let got = got.min((last + 1 - at) as usize);
                        if player.write_all(&buffer[..got]).is_err() {
                            return;
                        }
                        at += got as u64;
                        failed = 0;
                        if at > last {
                            return;
                        }
                    }
                    Err(_) => {
                        failed += 1;
                        break;
                    }
                }
            }
            if failed > TRIES {
                return;
            }
        }
    }
}

/// How much of a file's start is read to find its index: the header and
/// the index's own heading come well within it.
const HEAD: u64 = 64 * 1024;

/// Where a file keeps what a player needs to jump in it. A video file
/// from YouTube opens with its type (`ftyp`), its header (`moov`) and
/// then its index (`sidx`), one box after another.
#[derive(Debug, PartialEq, Eq)]
struct Layout {
    /// The last byte of the header.
    header_end: u64,
    /// The index's first and last byte.
    index: (u64, u64),
}

impl Layout {
    /// Reads the boxes at the start of a file: each is its length in
    /// four bytes, then its name in four. The index needn't be whole in
    /// `head`, only its heading.
    fn of(head: &[u8]) -> Option<Layout> {
        let mut at = 0usize;
        let mut header_end = None;
        loop {
            let heading = head.get(at..at + 8)?;
            let size = u32::from_be_bytes(heading[..4].try_into().ok()?) as usize;
            if size < 8 {
                return None;
            }
            match (&heading[4..], header_end) {
                (b"moov", _) => header_end = Some((at + size - 1) as u64),
                (b"sidx", Some(header_end)) => {
                    return Some(Layout {
                        header_end,
                        index: (at as u64, (at + size - 1) as u64),
                    });
                }
                (b"moof" | b"mdat", _) => return None,
                _ => {}
            }
            at += size;
        }
    }
}

/// `text` with what XML reserves written out.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// What the player asked for.
struct Asked {
    name: String,
    from: u64,
    to: Option<u64>,
    /// Whether it asked for a range, and so expects one back.
    ranged: bool,
}

impl Asked {
    /// Reads `GET /<name>` and its `Range`, if any.
    fn read(player: &TcpStream) -> Option<Asked> {
        let mut lines = BufReader::new(player).lines();
        let first = lines.next()?.ok()?;
        let name = first.strip_prefix("GET /")?.split(' ').next()?.to_string();
        let mut asked = Asked {
            name,
            from: 0,
            to: None,
            ranged: false,
        };
        for line in lines {
            let line = line.ok()?;
            if line.is_empty() {
                break;
            }
            let Some((header, value)) = line.split_once(':') else {
                continue;
            };
            if !header.eq_ignore_ascii_case("range") {
                continue;
            }
            let (from, to) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
            asked.from = from.parse().ok()?;
            asked.to = to.parse().ok();
            asked.ranged = true;
        }
        Some(asked)
    }
}

/// Whether the player closed its end. It sends nothing after its request,
/// so anything to read is the end of the connection.
fn hung_up(player: &TcpStream) -> bool {
    if player.set_nonblocking(true).is_err() {
        return true;
    }
    let closed = match player.peek(&mut [0]) {
        Ok(_) => true,
        Err(err) => err.kind() != io::ErrorKind::WouldBlock,
    };
    closed || player.set_nonblocking(false).is_err()
}

fn refuse(player: &mut TcpStream, status: StatusCode) -> io::Result<()> {
    player.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
    )
}

/// One file at YouTube, asked for a piece at a time.
struct Upstream<'a> {
    agent: &'a ureq::Agent,
    /// Where the file is. YouTube answers the address `yt-dlp` gives with
    /// a redirect; after the first piece this is where that led.
    url: String,
    /// The most a piece asks for.
    most: u64,
    /// How many pieces were asked for so far.
    asked: usize,
    /// The player asked for a stretch with an end, as it does with a
    /// manifest: it wants all of it, so it is asked for in one piece when
    /// it fits in one, not started small.
    bounded: bool,
    refused_waits: &'static [u64],
}

/// One piece as it arrives.
struct Piece {
    /// The length of the whole file.
    total: u64,
    kind: String,
    body: Option<Box<dyn Read>>,
}

impl Upstream<'_> {
    /// The piece an answer starts with, tried again when it may yet come:
    /// a request that didn't get through, and one YouTube refused, which
    /// it does to a new address for a moment ([`REFUSED_WAITS`]). Giving
    /// up here fails the whole stream.
    fn first_piece(&mut self, from: u64, to: Option<u64>) -> Result<Piece, StatusCode> {
        let mut unanswered = 0;
        let mut waits = self.refused_waits.iter();
        loop {
            let wait = match self.piece_from(from, to) {
                Err(status) if status.is_server_error() && unanswered < TRIES => {
                    unanswered += 1;
                    300
                }
                Err(StatusCode::FORBIDDEN) => match waits.next() {
                    Some(wait) => *wait,
                    None => return Err(StatusCode::FORBIDDEN),
                },
                piece => return piece,
            };
            self.asked = 0;
            std::thread::sleep(Duration::from_millis(wait));
        }
    }

    /// Asks for the piece that starts at `from` and goes no further than
    /// `to`. `Err` is the status to pass on to the player.
    fn piece_from(&mut self, from: u64, to: Option<u64>) -> Result<Piece, StatusCode> {
        let size = match FIRST_PIECES.get(self.asked) {
            Some(first) if !self.bounded => self.most.min(*first),
            _ => self.most,
        };
        self.asked += 1;
        let end = from + size - 1;
        let end = to.map_or(end, |to| to.min(end));
        let response = self
            .agent
            .get(&self.url)
            .header("Range", format!("bytes={from}-{end}"))
            .call()
            .map_err(|err| match err {
                ureq::Error::StatusCode(code) => {
                    StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY)
                }
                err => {
                    crate::log(format!("relay: no answer from YouTube ({err})"));
                    StatusCode::BAD_GATEWAY
                }
            })?;
        let total = total_length(response.status(), response.headers(), from)
            .ok_or(StatusCode::BAD_GATEWAY)?;
        let kind = response
            .headers()
            .get("content-type")
            .and_then(|kind| kind.to_str().ok())
            .unwrap_or("video/mp4")
            .to_string();
        self.url = response.get_uri().to_string();
        // No more than was asked for, whatever the answer holds.
        let body = response.into_body().into_reader().take(end - from + 1);
        Ok(Piece {
            total,
            kind,
            body: Some(Box::new(body)),
        })
    }
}

/// The whole file's length, from the answer to a request for the piece at
/// `from`: the number after the slash in `Content-Range: bytes 0-99/1234`.
/// `None` when the answer isn't that piece.
fn total_length(status: StatusCode, headers: &HeaderMap, from: u64) -> Option<u64> {
    if status != StatusCode::PARTIAL_CONTENT {
        return None;
    }
    let range = headers.get("content-range")?.to_str().ok()?;
    let (piece, total) = range.trim().strip_prefix("bytes ")?.split_once('/')?;
    let starts: u64 = piece.split_once('-')?.0.parse().ok()?;
    (starts == from).then(|| total.parse().ok())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the pretend YouTube does besides serving ranges of its file.
    #[derive(Default)]
    struct Quirks {
        /// Hangs up halfway through this many answers.
        cut_short: Mutex<usize>,
        /// Refuses this many requests before it answers.
        refused: Mutex<usize>,
        /// Says it is busy this many times before it answers.
        busy: Mutex<usize>,
        /// How many times it sent a request on from `/moved`.
        moved: Mutex<usize>,
        /// Every range it was asked for.
        asked: Mutex<Vec<(u64, u64)>>,
    }

    /// Serves `file` by range at `/file`, redirects `/moved` there, and
    /// refuses `/gone`. Returns its base address.
    fn youtube(file: Vec<u8>, quirks: Arc<Quirks>) -> String {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let file = Arc::new(file);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let (file, quirks) = (Arc::clone(&file), Arc::clone(&quirks));
                std::thread::spawn(move || {
                    let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
                    let first = lines.next().unwrap().unwrap();
                    let mut range = None;
                    for line in lines {
                        let line = line.unwrap();
                        if line.is_empty() {
                            break;
                        }
                        if let Some(asked) = line.to_ascii_lowercase().strip_prefix("range: bytes=")
                        {
                            let (from, to) = asked.split_once('-').unwrap();
                            range =
                                Some((from.parse::<u64>().unwrap(), to.parse::<u64>().unwrap()));
                        }
                    }
                    if first.starts_with("GET /moved ") {
                        let head = "HTTP/1.1 302 Found\r\nLocation: /file\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        *quirks.moved.lock().unwrap() += 1;
                        stream.write_all(head.as_bytes()).unwrap();
                        return;
                    }
                    if !first.starts_with("GET /file ") {
                        let head = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        stream.write_all(head.as_bytes()).unwrap();
                        return;
                    }
                    let refused = {
                        let mut left = quirks.refused.lock().unwrap();
                        let refused = *left > 0;
                        *left = left.saturating_sub(1);
                        refused
                    };
                    if refused {
                        let head = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        stream.write_all(head.as_bytes()).unwrap();
                        return;
                    }
                    let busy = {
                        let mut left = quirks.busy.lock().unwrap();
                        let busy = *left > 0;
                        *left = left.saturating_sub(1);
                        busy
                    };
                    if busy {
                        let head = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        stream.write_all(head.as_bytes()).unwrap();
                        return;
                    }
                    let (from, to) = range.expect("the relay always asks for a range");
                    quirks.asked.lock().unwrap().push((from, to));
                    let to = to.min(file.len() as u64 - 1);
                    let body = &file[from as usize..=to as usize];
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Range: bytes {from}-{to}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        file.len(),
                        body.len()
                    );
                    stream.write_all(head.as_bytes()).unwrap();
                    let cut = {
                        let mut left = quirks.cut_short.lock().unwrap();
                        let cut = *left > 0;
                        *left = left.saturating_sub(1);
                        cut
                    };
                    let body = if cut { &body[..body.len() / 2] } else { body };
                    let _ = stream.write_all(body);
                });
            }
        });
        base
    }

    fn file() -> Vec<u8> {
        (0..10_000u32).map(|n| (n % 251) as u8).collect()
    }

    /// [`file`], long enough to read a head from.
    fn file_bytes() -> Vec<u8> {
        (0..100_000u32).map(|n| (n % 251) as u8).collect()
    }

    /// What the relay answers to a request from `from` on, or to one
    /// without a range.
    fn get(
        address: &str,
        from: Option<u64>,
    ) -> Result<(u16, Option<String>, Vec<u8>), ureq::Error> {
        let mut request = ureq::get(address);
        if let Some(from) = from {
            request = request.header("Range", format!("bytes={from}-"));
        }
        let mut response = request.call()?;
        let range = response
            .headers()
            .get("content-range")
            .map(|range| range.to_str().unwrap().to_string());
        let body = response.body_mut().read_to_vec()?;
        Ok((response.status().as_u16(), range, body))
    }

    #[test]
    fn a_request_is_answered_from_pieces() {
        let quirks = Arc::new(Quirks::default());
        let base = youtube(file(), Arc::clone(&quirks));
        let relay = Relay::start(1000, &[10, 10]).unwrap();
        let address = relay.serve(&format!("{base}/file"));

        let (status, range, body) = get(&address, Some(2500)).unwrap();
        assert_eq!(status, 206);
        assert_eq!(range.as_deref(), Some("bytes 2500-9999/10000"));
        assert_eq!(body, file()[2500..]);
        let asked = quirks.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 8, "{asked:?}");
        assert_eq!(asked[0], (2500, 3499));
        assert_eq!(asked[7], (9500, 9999));

        // Without a range it is the whole file, as a plain answer.
        let (status, range, body) = get(&address, None).unwrap();
        assert_eq!((status, range), (200, None));
        assert_eq!(body, file());
    }

    #[test]
    fn a_redirect_is_followed_once_and_a_cut_piece_is_asked_for_again() {
        let quirks = Arc::new(Quirks::default());
        *quirks.cut_short.lock().unwrap() = 2;
        let base = youtube(file(), Arc::clone(&quirks));
        let relay = Relay::start(4000, &[10, 10]).unwrap();
        let address = relay.serve(&format!("{base}/moved"));

        let (_, _, body) = get(&address, Some(0)).unwrap();
        assert_eq!(body, file());
        let asked = quirks.asked.lock().unwrap().clone();
        // Two pieces came as halves, and each rest was asked for where
        // the half stopped.
        assert_eq!(asked, [(0, 3999), (2000, 5999), (4000, 7999), (8000, 9999)]);
    }

    #[test]
    fn an_answer_starts_with_small_pieces() {
        let quirks = Arc::new(Quirks::default());
        let big: Vec<u8> = (0..1_500_000u32).map(|n| (n % 251) as u8).collect();
        let base = youtube(big.clone(), Arc::clone(&quirks));
        let relay = Relay::start(PIECE, &[10, 10]).unwrap();
        let address = relay.serve(&format!("{base}/moved"));

        let (_, _, body) = get(&address, Some(0)).unwrap();
        assert!(body == big);
        let asked = quirks.asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            [(0, 131_071), (131_072, 1_179_647), (1_179_648, 1_499_999)]
        );
        // The redirect was followed for the first piece alone, and isn't
        // again for the next request.
        get(&address, Some(1_400_000)).unwrap();
        assert_eq!(*quirks.moved.lock().unwrap(), 1);
    }

    #[test]
    fn a_first_piece_that_didnt_get_through_is_asked_for_again() {
        let quirks = Arc::new(Quirks::default());
        *quirks.busy.lock().unwrap() = 2;
        let base = youtube(file(), Arc::clone(&quirks));
        let relay = Relay::start(1000, &[10, 10]).unwrap();
        let address = relay.serve(&format!("{base}/file"));
        let (status, _, body) = get(&address, Some(9000)).unwrap();
        assert_eq!(status, 206);
        assert_eq!(body, file()[9000..]);
    }

    #[test]
    fn an_address_refused_at_first_is_asked_for_again() {
        let quirks = Arc::new(Quirks::default());
        *quirks.refused.lock().unwrap() = 2;
        let base = youtube(file(), Arc::clone(&quirks));
        let relay = Relay::start(1000, &[10, 10]).unwrap();
        let address = relay.serve(&format!("{base}/file"));
        let (status, _, body) = get(&address, Some(0)).unwrap();
        assert_eq!(status, 206);
        assert_eq!(body, file());

        // Refused once more than it waits for, it is refused.
        *quirks.refused.lock().unwrap() = 3;
        assert!(matches!(
            get(&address, Some(0)),
            Err(ureq::Error::StatusCode(403))
        ));
    }

    /// The start of a file as YouTube lays it out: a type, a header of
    /// `header` bytes, an index of `index` bytes, then the video itself.
    fn laid_out(header: u32, index: u32) -> Vec<u8> {
        let mut file = Vec::new();
        for (name, size) in [
            (b"ftyp", 24),
            (b"moov", header),
            (b"sidx", index),
            (b"moof", 100),
        ] {
            file.extend(size.to_be_bytes());
            file.extend(name);
            file.resize(file.len() + size as usize - 8, 7);
        }
        file.resize(200_000, 9);
        file
    }

    #[test]
    fn the_index_is_found_after_the_header() {
        assert_eq!(
            Layout::of(&laid_out(700, 3000)),
            Some(Layout {
                header_end: 723,
                index: (724, 3723)
            })
        );
        // The index's heading is enough: the rest of it may come later.
        assert!(Layout::of(&laid_out(700, 3000)[..740]).is_some());
        assert_eq!(Layout::of(&laid_out(700, 3000)[..726]), None);
        // A file with no index before its video, or that isn't boxes.
        let mut plain = laid_out(700, 3000);
        plain[728..732].copy_from_slice(b"moof");
        assert_eq!(Layout::of(&plain), None);
        assert_eq!(Layout::of(&[0; 64]), None);
        assert_eq!(Layout::of(&[]), None);
    }

    #[test]
    fn a_manifest_says_where_each_files_index_is() {
        let quirks = Arc::new(Quirks::default());
        let base = youtube(laid_out(700, 3000), Arc::clone(&quirks));
        let relay = Relay::start(PIECE, &[10, 10]).unwrap();
        let track = |codec: &str| Track {
            url: format!("{base}/file"),
            codec: codec.into(),
            bits: 1_101_000,
        };
        let address = relay
            .manifest(&track("avc1.4D401F"), &track("mp4a.40.2"), 634.5)
            .unwrap();
        assert!(address.ends_with(".mpd"), "{address}");

        let mut response = ureq::get(&address).call().unwrap();
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/dash+xml"
        );
        let text = response.body_mut().read_to_string().unwrap();
        assert!(
            text.contains("mediaPresentationDuration=\"PT634.500S\""),
            "{text}"
        );
        assert_eq!(text.matches("indexRange=\"724-3723\"").count(), 2, "{text}");
        assert_eq!(text.matches("<Initialization range=\"0-723\"/>").count(), 2);
        assert!(text.contains("codecs=\"avc1.4D401F\"") && text.contains("codecs=\"mp4a.40.2\""));
        assert!(text.contains("bandwidth=\"1101000\""));

        // Each file is at an address of the relay's own, which answers a
        // stretch with an end in one piece, not started small.
        let file = text
            .split("<BaseURL>")
            .nth(1)
            .unwrap()
            .split('<')
            .next()
            .unwrap();
        assert!(file.starts_with(&format!("http://{}/", relay.at)), "{file}");
        quirks.asked.lock().unwrap().clear();
        let mut response = ureq::get(file)
            .header("Range", "bytes=1000-150999")
            .call()
            .unwrap();
        let body = response.body_mut().read_to_vec().unwrap();
        assert!(body == laid_out(700, 3000)[1000..151_000]);
        assert_eq!(*quirks.asked.lock().unwrap(), [(1000, 150_999)]);

        // A file laid out otherwise has no manifest.
        let other = youtube(file_bytes(), Arc::new(Quirks::default()));
        let plain = Track {
            url: format!("{other}/file"),
            codec: "avc1".into(),
            bits: 1,
        };
        assert!(relay.manifest(&plain, &plain, 10.0).is_err());
    }

    #[test]
    fn what_it_wasnt_given_or_youtube_refuses_is_refused() {
        let base = youtube(file(), Arc::new(Quirks::default()));
        let relay = Relay::start(1000, &[10, 10]).unwrap();
        let unknown = format!("http://{}/nothing", relay.at);
        assert!(matches!(
            get(&unknown, Some(0)),
            Err(ureq::Error::StatusCode(404))
        ));
        let gone = relay.serve(&format!("{base}/gone"));
        assert!(matches!(
            get(&gone, Some(0)),
            Err(ureq::Error::StatusCode(403))
        ));
        // Two addresses never share a name.
        assert_ne!(relay.serve("https://a"), relay.serve("https://a"));
    }

    #[test]
    fn the_length_comes_from_the_piece_asked_for() {
        let headers = |range: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("content-range", range.parse().unwrap());
            headers
        };
        let partial = StatusCode::PARTIAL_CONTENT;
        assert_eq!(
            total_length(partial, &headers("bytes 100-199/5000"), 100),
            Some(5000)
        );
        assert_eq!(
            total_length(partial, &headers("bytes 0-199/5000"), 100),
            None
        );
        assert_eq!(
            total_length(partial, &headers("bytes 100-199/*"), 100),
            None
        );
        assert_eq!(
            total_length(StatusCode::OK, &headers("bytes 100-199/5000"), 100),
            None
        );
        assert_eq!(total_length(partial, &HeaderMap::new(), 0), None);
    }
}
