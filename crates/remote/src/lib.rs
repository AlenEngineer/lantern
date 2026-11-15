//! A remote for lantern on a phone.
//!
//! [`Server`] serves one web page on the local network. The page shows the
//! [`Snapshot`] the app last [published](Server::publish) and sends back
//! [`Command`]s, which the app carries out as if a key had been pressed.
//! Nothing here knows the engine: the app says what there is to show and
//! decides what a command does.
//!
//! The page is open to anyone who can reach the port, but it is only a
//! page. What it shows and what it can do need the token, which the phone
//! gets from the address it was given (a QR code in the app's settings).
//!
//! ```text
//! GET  /            the page
//! GET  /events?t=…  the snapshot, again every time it changes (server-sent events)
//! POST /do?t=…      a command, as JSON
//! GET  /stage.jpg?t=…  a picture of the stage, when the app has one
//! ```
//!
//! The picture costs the app something to make, so it makes one only
//! while a phone keeps asking ([`Server::picture_wanted`]).

mod http;
mod protocol;

use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub use protocol::{
    Bang, Colors, Command, Found, Group, Hit, Live, Place, Plan, Slides, Snapshot, Step, StepKind,
    Timed, Upcoming,
};

/// The page, whole: its style and script are inside it.
const PAGE: &str = include_str!("page.html");

/// How long a phone that hears nothing waits for a sign of life. A phone
/// that left without saying so is found out by the write that fails.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// How long after a phone last asked for the picture the app goes on
/// making it. A phone showing it asks about once a second.
const PICTURE_WANTED: Duration = Duration::from_secs(4);

/// How long a connection may wait before it asks for anything. A phone's
/// browser opens connections ahead of time and uses them later; closed
/// under it, it gives up on the page after a few ("too many retries").
const IDLE: Duration = Duration::from_secs(120);

/// How long a request may take to arrive once it has started.
const REQUEST: Duration = Duration::from_secs(10);

/// The remote, listening. Dropping it stops it.
pub struct Server {
    shared: Arc<Shared>,
}

struct Shared {
    at: SocketAddr,
    token: String,
    /// What to do with a command. Called on the thread of the request.
    act: Box<dyn Fn(Command) + Send + Sync>,
    state: Mutex<State>,
    changed: Condvar,
    /// How many phones are listening for snapshots.
    watching: AtomicUsize,
    stopped: AtomicBool,
}

#[derive(Default)]
struct State {
    /// The last snapshot, as JSON. Empty until the first.
    snapshot: String,
    /// Goes up with every snapshot that differs from the one before.
    version: u64,
    /// The last picture of the stage, as a JPEG.
    picture: Option<Arc<Vec<u8>>>,
    /// When a phone last asked for it.
    picture_asked: Option<Instant>,
    /// The other machines that asked for anything, by address.
    reached: HashSet<IpAddr>,
}

impl Server {
    /// Starts listening on every address of this machine, at `port` (0 for
    /// any free one). Requests must carry `token`. `act` is called with
    /// each command a phone sends, on a thread of the server's.
    pub fn start(
        port: u16,
        token: impl Into<String>,
        act: impl Fn(Command) + Send + Sync + 'static,
    ) -> io::Result<Server> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))?;
        let shared = Arc::new(Shared {
            at: listener.local_addr()?,
            token: token.into(),
            act: Box::new(act),
            state: Mutex::default(),
            changed: Condvar::new(),
            watching: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
        });
        let serving = Arc::clone(&shared);
        std::thread::spawn(move || {
            for phone in listener.incoming().flatten() {
                if serving.stopped.load(Ordering::SeqCst) {
                    break;
                }
                let shared = Arc::clone(&serving);
                std::thread::spawn(move || shared.answer(phone));
            }
        });
        Ok(Server { shared })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.shared.at.port()
    }

    /// Shows every phone `snapshot`, if it differs from the last.
    pub fn publish(&self, snapshot: &Snapshot) {
        let Ok(json) = serde_json::to_string(snapshot) else {
            return;
        };
        let mut state = self.shared.state();
        if state.snapshot != json {
            state.snapshot = json;
            state.version += 1;
            self.shared.changed.notify_all();
        }
    }

    /// Gives every phone that shows it this picture of the stage, a JPEG.
    pub fn set_picture(&self, jpeg: Vec<u8>) {
        self.shared.state().picture = Some(Arc::new(jpeg));
    }

    /// Whether a phone is showing the picture, so one is worth making.
    pub fn picture_wanted(&self) -> bool {
        let asked = self.shared.state().picture_asked;
        asked.is_some_and(|asked| asked.elapsed() < PICTURE_WANTED)
    }

    /// How many other machines have asked this one for anything since it
    /// started. None, when a phone can't open the page, means the network
    /// keeps the phone from reaching this machine at all.
    pub fn reached(&self) -> usize {
        self.shared.state().reached.len()
    }

    /// How many phones are listening. With none, there is no one to
    /// publish for.
    pub fn watching(&self) -> usize {
        self.shared.watching.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.shared.changed.notify_all();
        // The listener is waiting for a phone: this one lets it see it
        // was stopped.
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, self.shared.at.port()));
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn answer(&self, mut phone: TcpStream) {
        // Its first byte may be long in coming; the rest follows at once.
        let _ = phone.set_read_timeout(Some(IDLE));
        if !matches!(phone.peek(&mut [0]), Ok(1)) {
            return;
        }
        let _ = phone.set_read_timeout(Some(REQUEST));
        let Some(request) = http::Request::read(&phone) else {
            // Not a request this speaks, such as a browser trying HTTPS
            // first: said plainly, so it goes on to plain HTTP.
            let _ = http::respond(
                &mut phone,
                "400 Bad Request",
                "text/plain",
                "plain HTTP only",
            );
            hang_up(phone);
            return;
        };
        if let Ok(peer) = phone.peer_addr()
            && !peer.ip().is_loopback()
        {
            self.state().reached.insert(peer.ip());
        }
        let allowed = request.query("t") == Some(self.token.as_str());
        let _ = match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => http::respond(&mut phone, "200 OK", "text/html; charset=utf-8", PAGE),
            (_, "/events" | "/do" | "/stage.jpg") if !allowed => {
                http::respond(&mut phone, "403 Forbidden", "text/plain", "wrong token")
            }
            ("GET", "/events") => self.events(&mut phone),
            ("GET", "/stage.jpg") => {
                let picture = {
                    let mut state = self.state();
                    state.picture_asked = Some(Instant::now());
                    state.picture.clone()
                };
                match picture {
                    Some(jpeg) => http::respond_bytes(&mut phone, "200 OK", "image/jpeg", &jpeg),
                    // Asking is what gets one made: the next try has it.
                    None => http::respond(&mut phone, "503 Service Unavailable", "text/plain", ""),
                }
            }
            ("POST", "/do") => match serde_json::from_slice::<Command>(&request.body) {
                Ok(command) => {
                    (self.act)(command);
                    http::respond(&mut phone, "204 No Content", "text/plain", "")
                }
                Err(err) => http::respond(
                    &mut phone,
                    "400 Bad Request",
                    "text/plain",
                    &format!("not a command: {err}"),
                ),
            },
            _ => http::respond(&mut phone, "404 Not Found", "text/plain", "nothing here"),
        };
        hang_up(phone);
    }

    /// Sends the snapshot now and again each time it changes, for as long
    /// as the phone stays.
    fn events(&self, phone: &mut TcpStream) -> io::Result<()> {
        phone.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        )?;
        self.watching.fetch_add(1, Ordering::SeqCst);
        // A phone that has just arrived: the app may have had no one to
        // publish for.
        (self.act)(Command::Hello);
        // With no snapshot kept, the one that greeting brings is awaited.
        let mut seen = {
            let state = self.state();
            if state.snapshot.is_empty() {
                state.version
            } else {
                0
            }
        };
        let sent = loop {
            let mut state = self.state();
            if state.version == seen && !self.stopped.load(Ordering::SeqCst) {
                state = self
                    .changed
                    .wait_timeout(state, HEARTBEAT)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            if self.stopped.load(Ordering::SeqCst) {
                break Ok(());
            }
            let event = if state.version == seen {
                ": still here\n\n".to_string()
            } else {
                seen = state.version;
                format!("data: {}\n\n", state.snapshot)
            };
            drop(state);
            if let Err(err) = phone
                .write_all(event.as_bytes())
                .and_then(|()| phone.flush())
            {
                break Err(err);
            }
        };
        // The last phone to leave takes the snapshot with it: the app stops
        // publishing with no one to see, so it would be stale for the next.
        if self.watching.fetch_sub(1, Ordering::SeqCst) == 1 {
            let mut state = self.state();
            state.snapshot.clear();
            state.picture = None;
        }
        sent
    }
}

/// Ends a connection so that the answer arrives: says it is done sending,
/// then waits a moment for the other end to finish. Closed outright with
/// something of the phone's still unread, the answer can be thrown away
/// with it.
fn hang_up(mut phone: TcpStream) {
    let _ = phone.shutdown(Shutdown::Write);
    let _ = phone.set_read_timeout(Some(Duration::from_millis(500)));
    let mut rest = [0; 1024];
    while matches!(phone.read(&mut rest), Ok(read) if read > 0) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::sync::mpsc;

    fn server() -> (Server, mpsc::Receiver<Command>) {
        let (sent, received) = mpsc::channel();
        let server = Server::start(0, "secret", move |command| {
            let _ = sent.send(command);
        })
        .unwrap();
        (server, received)
    }

    /// Sends `request` and returns the whole answer.
    fn ask(server: &Server, request: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    fn post(server: &Server, path: &str, body: &str) -> String {
        ask(
            server,
            &format!(
                "POST {path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    fn snapshot(title: &str) -> Snapshot {
        Snapshot {
            live: Some(Live {
                title: title.into(),
                ..Live::default()
            }),
            ..Snapshot::default()
        }
    }

    #[test]
    fn the_page_is_open_and_the_rest_needs_the_token() {
        let (server, commands) = server();
        let page = ask(&server, "GET / HTTP/1.1\r\n\r\n");
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
        assert!(page.contains("<!doctype html>"));

        for request in [
            "GET /events HTTP/1.1\r\n\r\n".to_string(),
            "GET /events?t=wrong HTTP/1.1\r\n\r\n".to_string(),
            "POST /do HTTP/1.1\r\nContent-Length: 13\r\n\r\n{\"do\":\"next\"}".to_string(),
        ] {
            let answer = ask(&server, &request);
            assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
        }
        assert!(ask(&server, "GET /elsewhere HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        assert!(commands.try_recv().is_err(), "nothing was carried out");
    }

    #[test]
    fn a_phone_that_arrives_later_isnt_shown_what_the_last_one_saw() {
        let (server, commands) = server();
        let watch = || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
            stream
                .write_all(b"GET /events?t=secret HTTP/1.1\r\n\r\n")
                .unwrap();
            BufReader::new(stream).lines()
        };
        let title = |lines: &mut std::io::Lines<BufReader<TcpStream>>| loop {
            let line = lines.next().unwrap().unwrap();
            if let Some(json) = line.strip_prefix("data: ") {
                let event: serde_json::Value = serde_json::from_str(json).unwrap();
                return event["live"]["title"].as_str().unwrap().to_string();
            }
        };
        server.publish(&snapshot("first"));
        let mut phone = watch();
        assert_eq!(title(&mut phone), "first");
        assert_eq!(commands.recv().unwrap(), Command::Hello);
        drop(phone);
        // The server finds out it left when a write to it fails, which
        // takes one or two.
        for unseen in 0.. {
            if server.watching() == 0 {
                break;
            }
            server.publish(&snapshot(&format!("unseen {unseen}")));
            std::thread::sleep(Duration::from_millis(5));
        }

        // The app answers the next phone's greeting with how things are.
        let mut phone = watch();
        assert_eq!(commands.recv().unwrap(), Command::Hello);
        server.publish(&snapshot("now"));
        assert_eq!(title(&mut phone), "now");
    }

    #[test]
    fn the_picture_is_made_only_while_asked_for() {
        let (server, _commands) = server();
        assert!(!server.picture_wanted());
        let none = ask(&server, "GET /stage.jpg?t=secret HTTP/1.1\r\n\r\n");
        assert!(none.starts_with("HTTP/1.1 503"), "{none}");
        assert!(server.picture_wanted(), "asking is what gets one made");

        server.set_picture(vec![0xff, 0xd8, 0xff, 0xd9]);
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        stream
            .write_all(b"GET /stage.jpg?t=secret HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        let head = String::from_utf8_lossy(&answer);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        assert!(head.contains("Content-Type: image/jpeg"));
        assert!(answer.ends_with(&[0xff, 0xd8, 0xff, 0xd9]));

        let refused = ask(&server, "GET /stage.jpg HTTP/1.1\r\n\r\n");
        assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
    }

    #[test]
    fn a_connection_opened_early_is_answered_when_it_asks() {
        let (server, _commands) = server();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        // Longer than a request may take, once started.
        std::thread::sleep(Duration::from_millis(300));
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        assert!(answer.ends_with("</html>\n"), "the whole page arrived");
    }

    #[test]
    fn what_isnt_http_is_told_so() {
        let (server, _commands) = server();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        // The start of a TLS greeting, as a browser trying HTTPS sends.
        stream
            .write_all(&[0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0xff, 0xfe, 0xfd, b'\n'])
            .unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        assert!(answer.starts_with(b"HTTP/1.1 400"), "{answer:?}");
        // Requests from this machine aren't a phone reaching it.
        assert_eq!(server.reached(), 0);
    }

    #[test]
    fn a_command_reaches_the_app() {
        let (server, commands) = server();
        let answer = post(&server, "/do?t=secret", r#"{"do":"next"}"#);
        assert!(answer.starts_with("HTTP/1.1 204"), "{answer}");
        assert_eq!(commands.recv().unwrap(), Command::Next);

        post(&server, "/do?t=secret", r#"{"do":"go","step":7}"#);
        assert_eq!(commands.recv().unwrap(), Command::Go { step: 7 });

        let answer = post(&server, "/do?t=secret", r#"{"do":"explode"}"#);
        assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn a_phone_is_sent_the_snapshot_and_each_change() {
        let (server, commands) = server();
        server.publish(&snapshot("Juan 3:16"));
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        stream
            .write_all(b"GET /events?t=secret HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
        let mut next_event = || loop {
            let line = lines.next().unwrap().unwrap();
            if let Some(json) = line.strip_prefix("data: ") {
                return serde_json::from_str::<serde_json::Value>(json).unwrap();
            }
        };
        assert_eq!(next_event()["live"]["title"], "Juan 3:16");
        // Arriving asks the app for a fresh snapshot.
        assert_eq!(commands.recv().unwrap(), Command::Hello);
        assert_eq!(server.watching(), 1);

        // The same again is not sent; a change is.
        server.publish(&snapshot("Juan 3:16"));
        server.publish(&snapshot("Salmos 23"));
        assert_eq!(next_event()["live"]["title"], "Salmos 23");

        drop(server);
        // Stopping lets the phone go: nothing more than the blank line
        // that ended the last event, then the end.
        assert!(lines.all(|line| line.is_ok_and(|line| line.is_empty())));
    }
}
