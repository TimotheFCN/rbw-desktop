//! One copy of the app at a time: launching it again brings forward the
//! one that runs, through a socket in the session folder, which only this
//! user can reach. A lock file says which copy runs, so that two launches
//! at once can't both start. The demo runs beside it.

use std::fs::{File, TryLockError};
use std::io::Write as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{App, Global};

use crate::window;

/// What a launch found.
pub enum Claim {
    /// This is the copy that runs.
    First(Instance),
    /// Another copy runs and was asked to show its window.
    HandedOver,
    /// No other copy could be reached, nor this one made reachable.
    Alone,
}

pub struct Instance {
    lock: File,
    listener: UnixListener,
    socket: PathBuf,
}

/// The lock, held for as long as the app runs.
struct Running(#[expect(dead_code, reason = "held, not read")] File);

impl Global for Running {}

/// How long a launch waits for a copy that is starting to listen.
const STARTING: Duration = Duration::from_secs(2);

/// Becomes the copy that runs, or asks the one that runs to show itself.
pub fn claim() -> Claim {
    let Ok(dir) = rbw_desktop_core::session_dir() else {
        return Claim::Alone;
    };
    let socket = dir.join("instance");
    let lock = match File::create(dir.join("instance.lock")) {
        Ok(lock) => lock,
        Err(error) => {
            log::warn!("the instance lock: {error}");
            return Claim::Alone;
        }
    };
    match lock.try_lock() {
        Ok(()) => {
            // A copy that ended without cleaning up left its socket.
            std::fs::remove_file(&socket).ok();
            match UnixListener::bind(&socket) {
                Ok(listener) => Claim::First(Instance {
                    lock,
                    listener,
                    socket,
                }),
                Err(error) => {
                    log::warn!("listening on {}: {error}", socket.display());
                    Claim::Alone
                }
            }
        }
        // Another copy runs, or is starting and listens soon.
        Err(TryLockError::WouldBlock) => {
            let step = Duration::from_millis(100);
            for _ in 0..STARTING.as_millis() / step.as_millis() {
                if UnixStream::connect(&socket)
                    .and_then(|mut stream| stream.write_all(b"show\n"))
                    .is_ok()
                {
                    return Claim::HandedOver;
                }
                std::thread::sleep(step);
            }
            Claim::Alone
        }
        Err(TryLockError::Error(error)) => {
            log::warn!("the instance lock: {error}");
            Claim::Alone
        }
    }
}

/// Shows the main window each time the app is launched again, and removes
/// the socket when the app quits.
pub fn listen(instance: Instance, cx: &mut App) {
    let Instance {
        lock,
        listener,
        socket,
    } = instance;
    cx.set_global(Running(lock));
    let (launched, mut launches) = futures::channel::mpsc::unbounded();
    let accepting = std::thread::Builder::new()
        .name("instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if stream.is_ok() && launched.unbounded_send(()).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = accepting {
        log::warn!("listening for launches: {error}");
        return;
    }
    cx.on_app_quit(move |_| {
        std::fs::remove_file(&socket).ok();
        async {}
    })
    .detach();
    cx.spawn(async move |cx| {
        while launches.next().await.is_some() {
            cx.update(|cx| {
                window::with_main_window(cx, |_, _, _| {});
                cx.activate(true);
            });
        }
    })
    .detach();
}
