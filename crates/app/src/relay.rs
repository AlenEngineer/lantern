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

/// A local address that serves `url` in pieces, or `None` when the relay
/// can't listen; the player then reads `url` itself, slower.
pub fn through(url: &str) -> Option<String> {
    static RELAY: OnceLock<Option<Arc<Relay>>> = OnceLock::new();
    let relay = RELAY.get_or_init(|| match Relay::start(PIECE) {
        Ok(relay) => Some(relay),
        Err(err) => {
            crate::log(format!(
                "relay: can't listen ({err}), streams read directly"
            ));
            None
        }
    });
    relay.as_ref().map(|relay| relay.serve(url))
}

struct Relay {
    at: SocketAddr,
    piece: u64,
    agent: ureq::Agent,
    /// The addresses served, by the name each is served under, oldest
    /// first. An address is replaced by where YouTube redirected it, so
    /// the redirect is followed once and not for every piece.
    served: Mutex<VecDeque<(String, String)>>,
}

impl Relay {
    /// Starts listening, on a thread of its own that lasts as long as
    /// the app.
    fn start(piece: u64) -> io::Result<Arc<Relay>> {
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
        let mut served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        let name = format!(
            "{:016x}{:x}",
            RandomState::new().build_hasher().finish(),
            served.len()
        );
        if served.len() == REMEMBERED {
            served.pop_front();
        }
        served.push_back((name.clone(), url.to_string()));
        format!("http://{}/{name}", self.at)
    }

    fn url_of(&self, name: &str) -> Option<String> {
        let served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        let found = served.iter().find(|(served, _)| served == name);
        found.map(|(_, url)| url.clone())
    }

    /// Remembers that what is served as `name` is at `url` now.
    fn moved(&self, name: &str, url: &str) {
        let mut served = self.served.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, old)) = served.iter_mut().find(|(served, _)| served == name) {
            *old = url.to_string();
        }
    }

    /// Answers one request of the player's: the file from where it asked
    /// to its end, fetched a piece at a time. The player hangs up when it
    /// jumps elsewhere, which ends this.
    fn answer(&self, mut player: TcpStream) {
        let Some(asked) = Asked::read(&player) else {
            return;
        };
        let Some(url) = self.url_of(&asked.name) else {
            let _ = refuse(&mut player, StatusCode::NOT_FOUND);
            return;
        };
        let mut upstream = Upstream {
            agent: &self.agent,
            url: url.clone(),
            most: self.piece,
            asked: 0,
        };
        let mut at = asked.from;
        let mut first = match upstream.first_piece(at, asked.to) {
            Ok(first) => first,
            Err(status) => {
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
}

/// One piece as it arrives.
struct Piece {
    /// The length of the whole file.
    total: u64,
    kind: String,
    body: Option<Box<dyn Read>>,
}

impl Upstream<'_> {
    /// The piece an answer starts with. What YouTube refuses is refused;
    /// a request that didn't get through is made again, since refusing
    /// the player here fails the whole stream.
    fn first_piece(&mut self, from: u64, to: Option<u64>) -> Result<Piece, StatusCode> {
        let mut tries = 0;
        loop {
            match self.piece_from(from, to) {
                Err(status) if status.is_server_error() && tries < TRIES => {
                    tries += 1;
                    self.asked = 0;
                    std::thread::sleep(Duration::from_millis(300));
                }
                piece => return piece,
            }
        }
    }

    /// Asks for the piece that starts at `from` and goes no further than
    /// `to`. `Err` is the status to pass on to the player.
    fn piece_from(&mut self, from: u64, to: Option<u64>) -> Result<Piece, StatusCode> {
        let size = FIRST_PIECES
            .get(self.asked)
            .map_or(self.most, |first| self.most.min(*first));
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
                _ => StatusCode::BAD_GATEWAY,
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
        let relay = Relay::start(1000).unwrap();
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
        let relay = Relay::start(4000).unwrap();
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
        let relay = Relay::start(PIECE).unwrap();
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
        let relay = Relay::start(1000).unwrap();
        let address = relay.serve(&format!("{base}/file"));
        let (status, _, body) = get(&address, Some(9000)).unwrap();
        assert_eq!(status, 206);
        assert_eq!(body, file()[9000..]);
    }

    #[test]
    fn what_it_wasnt_given_or_youtube_refuses_is_refused() {
        let base = youtube(file(), Arc::new(Quirks::default()));
        let relay = Relay::start(1000).unwrap();
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
