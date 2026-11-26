//! The verification code of the item shown, counting down: a view of its
//! own, so that each second of the countdown redraws its row alone rather
//! than the whole item.

use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, App, Context, EventEmitter, FontWeight, IntoElement, ParentElement as _, Render,
    Styled as _, Task, Window, div, px,
};
use rbw_desktop_core::{AccountId, Error, ItemId, VerificationCode};
use rbw_desktop_ui::{
    ActiveTheme as _, FieldRow, IconName, field_row, icon, motion, progress_ring,
};

use crate::backend;
use crate::clipboard;
use crate::recent;

/// How often the countdown moves.
const COUNTDOWN_STEP: Duration = Duration::from_secs(1);

pub(super) enum TotpEvent {
    /// The code was copied: the item was used.
    Copied,
    /// The item asks for the master password again before its next code.
    Relocked,
}

pub(super) struct TotpRow {
    account: AccountId,
    item: ItemId,
    code: VerificationCode,
    /// Whether a new code fades in: not the first, which came with its
    /// item.
    fades: bool,
    /// The code shown expired: the next one is fetched once the row is
    /// drawn again, so that a hidden window asks rbw for nothing.
    expired: bool,
    copied: bool,
    copied_timer: Task<()>,
    countdown: Task<()>,
    fetch_task: Task<()>,
}

impl EventEmitter<TotpEvent> for TotpRow {}

impl TotpRow {
    pub fn new(
        account: AccountId,
        item: ItemId,
        code: VerificationCode,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            account,
            item,
            code,
            fades: false,
            expired: false,
            copied: false,
            copied_timer: Task::ready(()),
            countdown: Task::ready(()),
            fetch_task: Task::ready(()),
        };
        this.count_down(cx);
        this
    }

    pub fn item(&self) -> &ItemId {
        &self.item
    }

    /// Shows another code of the item, counting down.
    pub fn set_code(&mut self, code: VerificationCode, cx: &mut Context<Self>) {
        self.code = code;
        self.expired = false;
        self.count_down(cx);
        cx.notify();
    }

    /// Redraws the row each second for the countdown, which a hidden window
    /// doesn't draw, then asks for the next code when drawn.
    fn count_down(&mut self, cx: &mut Context<Self>) {
        let expires_at = self.code.expires_at;
        self.countdown = cx.spawn(async move |this, cx| {
            while SystemTime::now() < expires_at {
                cx.background_executor().timer(COUNTDOWN_STEP).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            this.update(cx, |this, cx| {
                this.expired = true;
                cx.notify();
            })
            .ok();
        });
    }

    /// Fetches the next code, once the one shown expired. It doesn't count
    /// as using the item.
    fn fetch(&mut self, cx: &mut Context<Self>) {
        self.expired = false;
        self.fades = true;
        let (account, item) = (self.account.clone(), self.item.clone());
        self.fetch_task = backend::request(
            cx,
            move |backend| backend.verification_code(&account, &item),
            |this, result, cx| match result {
                Ok(code) => this.set_code(code, cx),
                Err(Error::RepromptRequired) => cx.emit(TotpEvent::Relocked),
                Err(_) => {}
            },
        );
    }

    /// Copies the code, as a click on its row does.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        clipboard::copy_secret(&self.code.code, Some(&self.account), cx);
        recent::used(&self.account, &self.item, cx);
        self.copied = true;
        self.copied_timer = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(motion::CONFIRMATION_HOLD)
                .await;
            this.update(cx, |this, cx| this.clear_copied(cx)).ok();
        });
        cx.emit(TotpEvent::Copied);
        cx.notify();
    }

    /// Something else of the item was copied: the row stops saying so.
    pub fn clear_copied(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.copied) {
            self.copied_timer = Task::ready(());
            cx.notify();
        }
    }
}

/// The verification code's row: its digits and a ring for the time left.
/// The item pane draws it too, empty, while the code is on its way.
pub(super) fn code_row(digits: AnyElement, fraction: f32, copied: bool, cx: &App) -> FieldRow {
    let tertiary = cx.theme().colors.text_tertiary;
    field_row(
        "code",
        "Verification code",
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(digits)
            .child(progress_ring(fraction, cx)),
    )
    .copied(copied)
    .action(icon(IconName::Copy).text_color(tertiary))
}

impl Render for TotpRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.expired {
            self.fetch(cx);
        }
        let code = &self.code;
        let remaining = code
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        let fraction = remaining.as_secs_f32() / code.period.as_secs_f32().max(1.);
        let digits = code.code.expose();
        let spaced = if digits.len() == 6 {
            format!("{} {}", &digits[..3], &digits[3..])
        } else {
            digits.to_string()
        };
        // Each new code fades up in place; the first comes with its item.
        let period = code
            .expires_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let digits = div().child(spaced.clone());
        let shown = if self.fades {
            motion::replace(("code", period), digits).into_any_element()
        } else {
            digits.into_any_element()
        };
        code_row(shown, fraction, self.copied, cx)
            .spoken_value(spaced)
            .on_click(cx.listener(|this, _, _, cx| this.copy(cx)))
    }
}
