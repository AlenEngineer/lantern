//! `lantern-web`: YouTube's own player in a window of its own, steered by
//! lantern through stdin and stdout (see the library's [`Command`] and
//! [`Event`] in `lantern-web-protocol`).
//!
//! The page is served from a loopback address and not loaded as a file:
//! YouTube's player needs to know which page it is on (the HTTP Referer)
//! and refuses to play without it.
//!
//! `lantern-web --pick <address> [--profile <folder>]` is the picker
//! instead ([`picker`]): YouTube's site, to pick videos from.

mod picker;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::thread;

use lantern_web_protocol::{Command, Event};
use tao::event::{Event as WindowEvent, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Fullscreen, WindowBuilder};
use wry::WebViewBuilder;

const PAGE: &str = include_str!("player.html");

enum Message {
    Command(Command),
    Page(String),
    /// stdin closed: the app is gone.
    Gone,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut pick = None;
    let mut profile = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pick" => pick = args.next(),
            "--profile" => profile = args.next().map(std::path::PathBuf::from),
            other => {
                eprintln!("lantern-web: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    if let Some(start) = pick {
        return picker::run(start, profile);
    }
    player();
}

/// YouTube's player over the stage, steered from stdin.
fn player() {
    let port = match serve() {
        Ok(port) => port,
        Err(err) => {
            eprintln!("lantern-web: can't serve the player page: {err}");
            std::process::exit(1);
        }
    };

    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("lantern-web")
        .with_decorations(false)
        .with_visible(false)
        .build(&event_loop)
        .expect("a window for the player");

    let page = event_loop.create_proxy();
    let builder = WebViewBuilder::new()
        .with_url(format!("http://127.0.0.1:{port}/"))
        .with_autoplay(true)
        .with_background_color((0, 0, 0, 255))
        .with_ipc_handler(move |request| {
            let _ = page.send_event(Message::Page(request.body().clone()));
        });
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("tao's GTK box"))
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    let webview = webview.expect("a web view for the player");

    read_commands(event_loop.create_proxy(), Message::Command, || {
        Message::Gone
    });

    // Commands that came before the page could take them.
    let mut waiting: Vec<Command> = Vec::new();
    let mut ready = false;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let message = match event {
            WindowEvent::NewEvents(StartCause::Init) => return,
            WindowEvent::UserEvent(message) => message,
            _ => return,
        };
        match message {
            Message::Gone | Message::Command(Command::Quit) => *flow = ControlFlow::Exit,
            Message::Page(json) => match serde_json::from_str::<Event>(&json) {
                Ok(event) => {
                    if event == Event::Ready {
                        ready = true;
                        for command in waiting.drain(..) {
                            let _ = webview.evaluate_script(&command.script());
                        }
                    }
                    tell(&event);
                }
                Err(err) => eprintln!("lantern-web: the page said something odd ({err}): {json}"),
            },
            Message::Command(command) => {
                match &command {
                    Command::Show { output, at } => {
                        let monitor = pick(&window, output.as_deref(), *at);
                        if monitor.is_none() && (output.is_some() || at.is_some()) {
                            tell(&Event::NoOutput);
                            return;
                        }
                        window.set_visible(true);
                        window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
                        window.set_always_on_top(true);
                        // A pointer over the player brings up YouTube's hover
                        // controls in front of the audience.
                        window.set_cursor_visible(false);
                    }
                    Command::Hide => window.set_visible(false),
                    _ => {}
                }
                if ready {
                    let _ = webview.evaluate_script(&command.script());
                } else {
                    waiting.push(command);
                }
            }
        }
    });
}

/// The monitor named `output`, or else the one holding the point `at`.
fn pick(
    window: &tao::window::Window,
    output: Option<&str>,
    at: Option<(i32, i32)>,
) -> Option<tao::monitor::MonitorHandle> {
    let monitors: Vec<_> = window.available_monitors().collect();
    let named = output.and_then(|name| {
        monitors
            .iter()
            .find(|m| m.name().as_deref() == Some(name))
            .cloned()
    });
    let held = || {
        let (x, y) = at?;
        monitors
            .iter()
            .find(|m| {
                let scale = m.scale_factor();
                let p = m.position().to_logical::<f64>(scale);
                let s = m.size().to_logical::<f64>(scale);
                let (x, y) = (f64::from(x), f64::from(y));
                x >= p.x && y >= p.y && x < p.x + s.width && y < p.y + s.height
            })
            .cloned()
    };
    let found = named.or_else(held);
    if found.is_none() && (output.is_some() || at.is_some()) {
        let seen: Vec<String> = monitors
            .iter()
            .map(|m| format!("{:?} at {:?} size {:?}", m.name(), m.position(), m.size()))
            .collect();
        eprintln!("lantern-web: no output {output:?} at {at:?}; there are {seen:?}");
    }
    found
}

/// One event, as one line on stdout.
fn tell(event: &Event) {
    let mut out = std::io::stdout().lock();
    let _ = serde_json::to_writer(&mut out, event);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

/// Reads commands from stdin on a thread of its own, each sent to the
/// event loop as `command(it)`, and `gone()` when stdin closes.
fn read_commands<M: Send + 'static>(
    proxy: EventLoopProxy<M>,
    command: fn(Command) -> M,
    gone: fn() -> M,
) {
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Command>(&line) {
                Ok(parsed) => {
                    if proxy.send_event(command(parsed)).is_err() {
                        return;
                    }
                }
                Err(err) => eprintln!("lantern-web: not a command ({err}): {line}"),
            }
        }
        let _ = proxy.send_event(gone());
    });
}

/// Serves the player page on a loopback port the system picks, and
/// nothing else.
fn serve() -> std::io::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || answer(stream));
        }
    });
    Ok(port)
}

fn answer(mut stream: TcpStream) {
    let mut first = String::new();
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    });
    if reader.read_line(&mut first).is_err() {
        return;
    }
    // Drain the headers; the page needs none of them.
    let mut line = String::new();
    while reader.read_line(&mut line).map(|n| n > 2).unwrap_or(false) {
        line.clear();
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let page = method == "GET" && (path == "/" || path.starts_with("/?"));
    let (status, body) = if page {
        ("200 OK", PAGE)
    } else {
        ("404 Not Found", "")
    };
    let kind = if page {
        "text/html; charset=utf-8"
    } else {
        "text/plain"
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
    let _ = reader.read(&mut [0u8; 0]);
}
