//! One copy of the app at a time: launching it again brings forward the
//! one that runs, through a socket in the session folder, which only this
//! user can reach. The demo runs beside it.

use std::io::Write as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use futures::StreamExt as _;
use gpui::App;

use crate::window;

fn socket() -> Option<PathBuf> {
    Some(rbw_desktop_core::session_dir().ok()?.join("instance"))
}

/// Asks the copy that runs to show its window, and says whether one did.
pub fn hand_over() -> bool {
    socket()
        .and_then(|path| UnixStream::connect(path).ok())
        .is_some_and(|mut stream| stream.write_all(b"show\n").is_ok())
}

/// Shows the main window each time the app is launched again.
pub fn listen(cx: &mut App) {
    let Some(path) = socket() else { return };
    // A copy that ended without cleaning up left its socket; none answered
    // on it.
    std::fs::remove_file(&path).ok();
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("listening on {}: {error}", path.display());
            return;
        }
    };
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
