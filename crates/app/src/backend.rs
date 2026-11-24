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
    match error {
        Error::Locked => "The vault is locked.",
        Error::NotSignedIn => "This account is signed out.",
        Error::WrongPassword => "Incorrect password. Try again.",
        Error::WrongCode => "That code isn't right. Try again.",
        Error::WrongApiKey => "That API key isn't right.",
        Error::Offline => "The server can't be reached. Check your connection.",
        Error::WrongServer => "There's no Bitwarden server at this address. Check it.",
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
        _ => "Something went wrong.",
    }
}
