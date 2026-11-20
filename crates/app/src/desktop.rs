//! What Lantern asks of the desktop while it is showing: not to turn the
//! screens off, and not to put notifications over the service.
//!
//! Both are asked over the session bus and held by the connection, so a
//! Lantern that dies lets go of them: the screens' by
//! `org.freedesktop.ScreenSaver`, the notifications' by the `Inhibit` that
//! KDE added to `org.freedesktop.Notifications`. A desktop without one
//! says so once in the log and nothing else happens. For notification
//! daemons without `Inhibit`, the config names a command to run as a
//! service starts and one as it ends.

use std::sync::mpsc::Sender;

/// Something held from the desktop while Lantern needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    /// The screens stay on.
    Awake,
    /// Notifications wait.
    Quiet,
}

/// What is held now, and the thread that asks for it. Asking never waits
/// on the bus: the dashboard goes on while the desktop answers.
#[derive(Default)]
pub struct Desktop {
    awake: bool,
    quiet: bool,
    to: Option<Sender<(Ask, bool)>>,
}

impl Desktop {
    /// The screens stay on while `awake`.
    pub fn keep_awake(&mut self, awake: bool) {
        if awake != self.awake {
            self.awake = awake;
            self.send(Ask::Awake, awake);
        }
    }

    /// Notifications wait while `quiet`. `true` when that just changed,
    /// for the commands the config names.
    pub fn keep_quiet(&mut self, quiet: bool) -> bool {
        if quiet == self.quiet {
            return false;
        }
        self.quiet = quiet;
        self.send(Ask::Quiet, quiet);
        true
    }

    fn send(&mut self, ask: Ask, hold: bool) {
        // A test that starts a service would otherwise hold the screens
        // of whoever runs it.
        if cfg!(test) {
            return;
        }
        let to = self.to.get_or_insert_with(bus::start);
        let _ = to.send((ask, hold));
    }
}

/// Runs `command` in a shell (`sh`, or `cmd` on Windows), without waiting
/// for it: a command that hangs holds nothing up, and one started as
/// Lantern quits still ends.
pub fn run(command: &str) {
    crate::log(format!("desktop: running `{command}`"));
    let (shell, flag) = match cfg!(windows) {
        true => ("cmd", "/C"),
        false => ("sh", "-c"),
    };
    match crate::programs::command(shell)
        .arg(flag)
        .arg(command)
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                if let Ok(status) = child.wait()
                    && !status.success()
                {
                    crate::log(format!("desktop: the command ended with {status}"));
                }
            });
        }
        Err(error) => crate::log(format!("desktop: the command did not start: {error}")),
    }
}

#[cfg(target_os = "linux")]
mod bus {
    use std::sync::mpsc::{self, Sender};

    use zbus::blocking::Connection;

    use super::Ask;

    const WHY_AWAKE: &str = "Something is on the projector";
    const WHY_QUIET: &str = "A service is running";

    pub fn start() -> Sender<(Ask, bool)> {
        let (to, from) = mpsc::channel::<(Ask, bool)>();
        std::thread::spawn(move || {
            let bus = match Connection::session() {
                Ok(bus) => bus,
                Err(error) => {
                    crate::log(format!("desktop: no session bus: {error}"));
                    return;
                }
            };
            let (mut awake, mut quiet) = (None, None);
            for (ask, hold) in from {
                let held = match ask {
                    Ask::Awake => &mut awake,
                    Ask::Quiet => &mut quiet,
                };
                match (hold, held.take()) {
                    (true, None) => *held = inhibit(&bus, ask),
                    (true, cookie) => *held = cookie,
                    (false, Some(cookie)) => release(&bus, ask, cookie),
                    (false, None) => {}
                }
            }
        });
        to
    }

    fn service(ask: Ask) -> (&'static str, &'static str) {
        match ask {
            Ask::Awake => (
                "org.freedesktop.ScreenSaver",
                "/org/freedesktop/ScreenSaver",
            ),
            Ask::Quiet => (
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
            ),
        }
    }

    fn inhibit(bus: &Connection, ask: Ask) -> Option<u32> {
        let (name, path) = service(ask);
        let reply = match ask {
            Ask::Awake => bus.call_method(
                Some(name),
                path,
                Some(name),
                "Inhibit",
                &("Lantern", WHY_AWAKE),
            ),
            Ask::Quiet => {
                let hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
                bus.call_method(
                    Some(name),
                    path,
                    Some(name),
                    "Inhibit",
                    &("lantern", WHY_QUIET, hints),
                )
            }
        };
        match reply.and_then(|reply| reply.body().deserialize::<u32>()) {
            Ok(cookie) => {
                crate::log(format!("desktop: {ask:?} is held"));
                Some(cookie)
            }
            // A notification service without `Inhibit` lands here: the
            // config's commands are the way for those.
            Err(error) => {
                crate::log(format!("desktop: {ask:?} can't be held here: {error}"));
                None
            }
        }
    }

    fn release(bus: &Connection, ask: Ask, cookie: u32) {
        let (name, path) = service(ask);
        match bus.call_method(Some(name), path, Some(name), "UnInhibit", &(cookie,)) {
            Ok(_) => crate::log(format!("desktop: {ask:?} is let go")),
            Err(error) => crate::log(format!("desktop: {ask:?} was not let go: {error}")),
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod bus {
    use std::sync::mpsc::{self, Sender};

    use super::Ask;

    /// Nothing is asked: the messages go nowhere.
    pub fn start() -> Sender<(Ask, bool)> {
        mpsc::channel().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_only_when_quiet_changes() {
        let mut desktop = Desktop::default();
        assert!(!desktop.keep_quiet(false));
        assert!(desktop.keep_quiet(true));
        assert!(!desktop.keep_quiet(true));
        assert!(desktop.keep_quiet(false));
    }
}
