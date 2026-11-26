//! How views reach rbw. Every backend call blocks, so it runs on the
//! background executor and its result comes back to the view on the main
//! thread.

use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Global, Task, Window};
use rbw_desktop_core::{Backend, Error, RbwBackend, Result};

use crate::demo;

#[derive(Clone)]
struct Shared(Arc<dyn Backend>);

impl Global for Shared {}

/// The backend the app runs on: rbw, or the demo's when `RBW_DESKTOP_DEMO`
/// is set. It fails when rbw is missing or too old. It blocks while it
/// asks rbw its version.
pub fn connect() -> Result<Arc<dyn Backend>> {
    match demo::words() {
        Some(words) => Ok(Arc::new(demo::backend(&words)?)),
        None => Ok(Arc::new(RbwBackend::new()?)),
    }
}

pub fn init(backend: Arc<dyn Backend>, cx: &mut App) {
    cx.set_global(Shared(backend));
}

/// Whether there is a backend to ask, which there isn't while rbw is
/// missing.
pub fn ready(cx: &App) -> bool {
    cx.has_global::<Shared>()
}

/// Runs `call` off the main thread, then `then` with its result on the view
/// that asked, if the view still exists.
///
/// Keep the returned task in the view while the answer still matters:
/// dropping it cancels the delivery, so a newer request replacing an older
/// one in the same field makes the older answer go nowhere.
pub fn request<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
    then: impl FnOnce(&mut V, Result<T>, &mut Context<V>) + 'static,
) -> Task<()> {
    let backend = cx.global::<Shared>().0.clone();
    let work = cx.background_spawn(async move { call(backend.as_ref()) });
    cx.spawn(async move |view, cx| {
        let result = work.await;
        view.update(cx, |view, cx| then(view, result, cx)).ok();
    })
}

/// [`request`] for an answer that needs the window, such as one that sets a
/// text field's value or moves the focus.
pub fn request_in<V: 'static, T: Send + 'static>(
    window: &Window,
    cx: &mut Context<V>,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
    then: impl FnOnce(&mut V, Result<T>, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    let backend = cx.global::<Shared>().0.clone();
    let work = cx.background_spawn(async move { call(backend.as_ref()) });
    cx.spawn_in(window, async move |view, cx| {
        let result = work.await;
        view.update_in(cx, |view, window, cx| then(view, result, window, cx))
            .ok();
    })
}

/// [`request`] on a thread of its own rather than the background executor,
/// for a call that can wait on the network for a long time, such as a
/// website's icon, so that it never holds up the calls people wait for.
/// Dropping the task drops the answer; the call still runs to its end.
pub fn request_on_thread<V: 'static, T: Send + 'static>(
    name: &str,
    cx: &mut Context<V>,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
    then: impl FnOnce(&mut V, Result<T>, &mut Context<V>) + 'static,
) -> Task<()> {
    let answer = on_thread(name, cx.global::<Shared>().0.clone(), call);
    cx.spawn(async move |view, cx| {
        let result = answer.await;
        view.update(cx, |view, cx| then(view, result, cx)).ok();
    })
}

/// [`request`] for a call that waits on the server, such as signing in,
/// syncing or saving an item: it runs on a thread of its own, so that
/// however long the server takes, the quick local calls on the background
/// executor never queue behind it.
pub fn request_network<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
    then: impl FnOnce(&mut V, Result<T>, &mut Context<V>) + 'static,
) -> Task<()> {
    request_on_thread(NETWORK_THREAD, cx, call, then)
}

/// [`request_network`] for an answer that needs the window, as
/// [`request_in`].
pub fn request_network_in<V: 'static, T: Send + 'static>(
    window: &Window,
    cx: &mut Context<V>,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
    then: impl FnOnce(&mut V, Result<T>, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    let answer = on_thread(NETWORK_THREAD, cx.global::<Shared>().0.clone(), call);
    cx.spawn_in(window, async move |view, cx| {
        let result = answer.await;
        view.update_in(cx, |view, window, cx| then(view, result, window, cx))
            .ok();
    })
}

/// The name of the threads that wait on the server.
const NETWORK_THREAD: &str = "rbw network";

/// Runs `call` on a new thread named `name`; the future gives its answer.
fn on_thread<T: Send + 'static>(
    name: &str,
    backend: Arc<dyn Backend>,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
) -> impl std::future::Future<Output = Result<T>> + Send + 'static {
    let (send, answer) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || send.send(call(backend.as_ref())));
    if let Err(error) = &spawned {
        log::warn!("starting a thread: {error}");
    }
    async move {
        answer
            .await
            .unwrap_or_else(|_| Err(Error::Other("the call's thread ended".into())))
    }
}

/// Runs `call` off the main thread, for an answer that no view waits for.
pub fn call<T: Send + 'static>(
    cx: &mut App,
    call: impl FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
) -> Task<Result<T>> {
    let backend = cx.global::<Shared>().0.clone();
    cx.background_spawn(async move { call(backend.as_ref()) })
}

/// Runs `call` off the main thread when nothing waits for its answer, such
/// as removing an account that was abandoned. A failure is only logged.
pub fn forget(cx: &mut App, call: impl FnOnce(&dyn Backend) -> Result<()> + Send + 'static) {
    let backend = cx.global::<Shared>().0.clone();
    cx.background_spawn(async move {
        if let Err(error) = call(backend.as_ref()) {
            log::warn!("{error}");
        }
    })
    .detach();
}

/// Runs `call` on this thread, waiting for it: only while the app quits,
/// when nothing is drawn anymore and the call must finish before the end.
/// A failure is only logged.
pub fn block(cx: &App, call: impl FnOnce(&dyn Backend) -> Result<()>) {
    if let Err(error) = call(cx.global::<Shared>().0.as_ref()) {
        log::warn!("{error}");
    }
}

/// What a view shows while its data is on the way.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(Error),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(value) => Some(value),
            Load::Loading | Load::Failed(_) => None,
        }
    }
}

impl<T> From<Result<T>> for Load<T> {
    fn from(result: Result<T>) -> Self {
        match result {
            Ok(value) => Load::Ready(value),
            Err(error) => Load::Failed(error),
        }
    }
}

/// A sentence for people about what went wrong. The errors' own `Display`
/// text is for logs.
pub fn describe(error: &Error) -> &'static str {
    if matches!(
        error,
        Error::InvalidItem(_) | Error::Server(_) | Error::Other(_)
    ) {
        log_once(error);
    }
    match error {
        Error::Locked => "The vault is locked.",
        Error::NotSignedIn => "This account is signed out.",
        Error::WrongPassword => "Incorrect password. Try again.",
        Error::WrongCode => "That code isn't right. Try again.",
        Error::WrongApiKey => "That API key isn't right.",
        Error::Offline => "The server can't be reached. Check your connection.",
        Error::WrongServer => "There's no Bitwarden server at the account's address.",
        Error::Timeout => "rbw is taking too long to answer.",
        Error::RbwNotFound => "rbw isn't installed.",
        Error::RbwTooOld { .. } => "rbw needs updating.",
        Error::NotFound => "This item no longer exists.",
        Error::Conflict => "The item changed on another device. Sync, then try again.",
        Error::ReadOnly => "This item can't be changed.",
        Error::InvalidItem(_) => "Some details of the item aren't valid.",
        Error::Server(_) => "The server refused the request.",
        Error::PasswordRequired | Error::RepromptRequired => {
            "This needs your master password again."
        }
        error => {
            log_once(error);
            "Something went wrong."
        }
    }
}

/// Logs what an error says that its sentence for people leaves out, such
/// as the server's own message, once while it stays the same: a view may
/// describe the same error each time it draws.
fn log_once(error: &Error) {
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let text = error.to_string();
    let Ok(mut last) = LAST.lock() else { return };
    if last.as_deref() != Some(text.as_str()) {
        log::warn!("backend error: {text}");
        *last = Some(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_error_people_meet_has_its_sentence() {
        let generic = describe(&Error::Other(String::new()));
        let errors = [
            Error::Locked,
            Error::NotSignedIn,
            Error::WrongPassword,
            Error::WrongCode,
            Error::WrongApiKey,
            Error::Offline,
            Error::WrongServer,
            Error::Timeout,
            Error::RbwNotFound,
            Error::NotFound,
            Error::Conflict,
            Error::ReadOnly,
            Error::InvalidItem(String::new()),
            Error::Server(String::new()),
            Error::PasswordRequired,
            Error::RepromptRequired,
        ];
        let mut sentences: Vec<_> = errors.iter().map(describe).collect();
        assert!(sentences.iter().all(|sentence| *sentence != generic));
        assert!(sentences.iter().all(|sentence| sentence.ends_with('.')));
        sentences.dedup();
        // the two that ask for the master password say the same
        assert_eq!(sentences.len(), errors.len() - 1);
    }
}
