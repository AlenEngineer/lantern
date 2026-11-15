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
//! ```

mod http;
mod protocol;

use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use protocol::{
    Colors, Command, Live, Place, Plan, Slides, Snapshot, Step, StepKind, Timed, Upcoming,
};

/// The page, whole: its style and script are inside it.
const PAGE: &str = include_str!("page.html");

/// How long a phone that hears nothing waits for a sign of life. A phone
/// that left without saying so is found out by the write that fails.
const HEARTBEAT: Duration = Duration::from_secs(15);

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
        let _ = phone.set_read_timeout(Some(Duration::from_secs(10)));
        let Some(request) = http::Request::read(&phone) else {
            return;
        };
        let allowed = request.query("t") == Some(self.token.as_str());
        let _ = match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => http::respond(&mut phone, "200 OK", "text/html; charset=utf-8", PAGE),
            (_, "/events" | "/do") if !allowed => {
                http::respond(&mut phone, "403 Forbidden", "text/plain", "wrong token")
            }
            ("GET", "/events") => self.events(phone),
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
    }

    /// Sends the snapshot now and again each time it changes, for as long
    /// as the phone stays.
    fn events(&self, mut phone: TcpStream) -> io::Result<()> {
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
            self.state().snapshot.clear();
        }
        sent
    }
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
