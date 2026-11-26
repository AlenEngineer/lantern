//! `lantern-web --pick <address>`: YouTube's own site in a window on the
//! operator's screen, for picking videos without a key.
//!
//! Two web views share the window: lantern's strip on top (what was picked,
//! and Done) and YouTube's page under it, which lantern doesn't draw on. A
//! video the operator clicks isn't opened: its id is told to the app as
//! [`Event::Picked`]. Three ways a video can be asked for are caught:
//!
//! - a click on a link to it, before YouTube's page handles the click (its
//!   own navigation within the page never loads a new address);
//! - a load of a video's address (typed, or a link YouTube loads as a page);
//! - a video opened in a new window (middle click).
//!
//! The window stays on YouTube, and Google's sign-in and cookie pages:
//! other addresses don't load. Its cookies are kept in `profile`, so
//! YouTube's cookie question is answered once.

use std::path::PathBuf;
use std::rc::Rc;

use lantern_web_protocol::{Command, Event, is_video_id};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event as WindowEvent, WindowEvent as Happened};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use url::Url;
use wry::{NewWindowResponse, Rect, WebContext, WebViewBuilder};

use crate::{read_commands, tell};

const STRIP: &str = include_str!("strip.html");

/// How tall lantern's strip is, in logical pixels.
const STRIP_HEIGHT: f64 = 48.0;

/// Caught in YouTube's page before YouTube handles it: a click on a link to
/// a video. Only the id is told; the click goes no further.
const CATCH: &str = r#"
document.addEventListener("click", (e) => {
  const a = e.target && e.target.closest ? e.target.closest("a[href]") : null;
  if (!a) return;
  let url;
  try { url = new URL(a.href, location.href); } catch (_) { return; }
  if (!/(^|\.)youtube\.com$/.test(url.hostname)) return;
  let id = null;
  if (url.pathname === "/watch") id = url.searchParams.get("v");
  const short = url.pathname.match(/^\/shorts\/([A-Za-z0-9_-]{11})/);
  if (short) id = short[1];
  if (!id || !/^[A-Za-z0-9_-]{11}$/.test(id)) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  window.ipc.postMessage(JSON.stringify({ picked: id }));
}, true);
"#;

enum Message {
    Command(Command),
    Picked(String),
    Done,
    /// stdin closed: the app is gone.
    Gone,
}

/// The video `address` is a link to, if it is one.
pub fn video(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    lantern_source_link::youtube_id(&url).filter(|id| is_video_id(id))
}

/// Whether the picker's window may load `address`: YouTube, and the
/// Google pages YouTube sends to for signing in and for cookies.
pub fn allowed(address: &str) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    if url.scheme() != "https" {
        return url.scheme() == "about";
    }
    let host = url.host_str().unwrap_or("");
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    under("youtube.com") || under("google.com") || host == "youtu.be"
}

/// Runs the picker on YouTube's page at `start` until Done.
pub fn run(start: String, profile: Option<PathBuf>) {
    let event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("YouTube · lantern")
        .with_inner_size(LogicalSize::new(1120.0, 760.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .build(&event_loop)
        .expect("a window for the picker");
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let (strip_rect, page_rect) = rects(size.width, size.height);

    let done = event_loop.create_proxy();
    let strip = WebViewBuilder::new()
        .with_html(STRIP)
        .with_bounds(strip_rect)
        .with_ipc_handler(move |request| {
            if request.body().contains("\"done\"") {
                let _ = done.send_event(Message::Done);
            }
        });

    let mut context = WebContext::new(profile);
    let (by_click, by_load, by_window) = (
        event_loop.create_proxy(),
        event_loop.create_proxy(),
        event_loop.create_proxy(),
    );
    let page = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(start)
        .with_bounds(page_rect)
        .with_initialization_script_for_main_only(CATCH, true)
        .with_ipc_handler(move |request| {
            // YouTube's page could send anything here: only an id is taken.
            let id = serde_json::from_str::<serde_json::Value>(request.body())
                .ok()
                .and_then(|v| v.get("picked")?.as_str().map(str::to_string))
                .filter(|id| is_video_id(id));
            if let Some(id) = id {
                let _ = by_click.send_event(Message::Picked(id));
            }
        })
        .with_navigation_handler(move |address| match video(&address) {
            Some(id) => {
                let _ = by_load.send_event(Message::Picked(id));
                false
            }
            None => allowed(&address),
        })
        .with_new_window_req_handler(move |address, _| {
            if let Some(id) = video(&address) {
                let _ = by_window.send_event(Message::Picked(id));
            }
            NewWindowResponse::Deny
        });

    // On Linux the window's own GTK box lays the views out: the strip at
    // its height on top, YouTube's page taking the rest, through every
    // resize (a tiling desktop sizes the window after it opens).
    #[cfg(target_os = "linux")]
    let views = {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        use wry::{WebViewBuilderExtUnix, WebViewExtUnix};
        let vbox = window.default_vbox().expect("tao's GTK box");
        let strip = strip
            .build_gtk(vbox)
            .expect("a web view for lantern's strip");
        let page = page.build_gtk(vbox).expect("a web view for YouTube");
        let bar = strip.webview();
        bar.set_size_request(-1, STRIP_HEIGHT as i32);
        vbox.set_child_packing(&bar, false, false, 0, gtk::PackType::Start);
        Rc::new((strip, page))
    };
    #[cfg(not(target_os = "linux"))]
    let views = Rc::new((
        strip
            .build_as_child(&window)
            .expect("a web view for lantern's strip"),
        page.build_as_child(&window)
            .expect("a web view for YouTube"),
    ));

    read_commands(event_loop.create_proxy(), Message::Command, || {
        Message::Gone
    });
    let mut picked = 0usize;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        // The context holds the profile for the page's whole life.
        let _ = &context;
        let message = match event {
            // On Linux GTK lays the views out (above).
            #[cfg(not(target_os = "linux"))]
            WindowEvent::WindowEvent {
                event: Happened::Resized(size),
                ..
            } => {
                let size = size.to_logical::<f64>(window.scale_factor());
                let (strip_rect, page_rect) = rects(size.width, size.height);
                let _ = views.0.set_bounds(strip_rect);
                let _ = views.1.set_bounds(page_rect);
                return;
            }
            WindowEvent::WindowEvent {
                event: Happened::CloseRequested,
                ..
            } => Message::Done,
            WindowEvent::UserEvent(message) => message,
            _ => return,
        };
        match message {
            Message::Picked(id) => {
                picked += 1;
                tell(&Event::Picked { id });
                // Until the app names it, the count says it was taken.
                let text = format!("{picked} picked");
                let _ = views.0.evaluate_script(&Command::Strip { text }.script());
            }
            Message::Command(command @ Command::Strip { .. }) => {
                let _ = views.0.evaluate_script(&command.script());
            }
            Message::Command(Command::Quit) | Message::Gone => *flow = ControlFlow::Exit,
            Message::Command(_) => {}
            Message::Done => {
                tell(&Event::Done);
                *flow = ControlFlow::Exit;
            }
        }
    });
}

/// Where the strip and YouTube's page go in a window `width`×`height`.
fn rects(width: f64, height: f64) -> (Rect, Rect) {
    let strip = Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(width, STRIP_HEIGHT).into(),
    };
    let page = Rect {
        position: LogicalPosition::new(0.0, STRIP_HEIGHT).into(),
        size: LogicalSize::new(width, (height - STRIP_HEIGHT).max(1.0)).into(),
    };
    (strip, page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_to_a_video_are_caught_and_others_pass() {
        let id = Some("jNQXAC9IVRw".to_string());
        assert_eq!(
            video("https://www.youtube.com/watch?v=jNQXAC9IVRw&t=4s"),
            id
        );
        assert_eq!(video("https://m.youtube.com/watch?v=jNQXAC9IVRw"), id);
        assert_eq!(video("https://www.youtube.com/shorts/jNQXAC9IVRw"), id);
        assert_eq!(video("https://youtu.be/jNQXAC9IVRw"), id);
        assert_eq!(
            video("https://www.youtube.com/results?search_query=himno"),
            None
        );
        assert_eq!(video("https://www.youtube.com/@Channel"), None);
        assert_eq!(video("https://example.com/watch?v=jNQXAC9IVRw"), None);
        assert_eq!(video("not a link"), None);
    }

    #[test]
    fn the_window_stays_on_youtube_and_googles_sign_in() {
        assert!(allowed("https://www.youtube.com/results?search_query=x"));
        assert!(allowed("https://consent.youtube.com/m?continue=x"));
        assert!(allowed("https://accounts.google.com/ServiceLogin"));
        assert!(allowed("about:blank"));
        assert!(!allowed("https://example.com/"));
        assert!(!allowed("https://notyoutube.com/"));
        assert!(!allowed("http://www.youtube.com/"), "not over plain http");
        assert!(!allowed("file:///etc/passwd"));
    }
}
