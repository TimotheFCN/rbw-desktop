//! Accounts: each account on this computer with its server and state, to
//! sign in or out of, and Touch ID for each on a Mac that has it; and
//! adding another, which the main window does.

use gpui::{
    AnyElement, App, Context, ElementId, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, px, relative,
};
use rbw_desktop_core::{Account, LockState, Server};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, avatar, button, card, setting_row, switch,
};

use super::store::Store;
use crate::accounts::{self, Accounts};
use crate::alerts;
use crate::backend;
use crate::window;

pub struct AccountsPage {
    store: Entity<Store>,
    _subscriptions: [Subscription; 2],
}

impl AccountsPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscriptions: [
                cx.observe(&Accounts::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&store, |_, _, cx| cx.notify()),
            ],
            store,
        }
    }

    fn render_account(&self, account: &Account, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let dim = theme.colors.text_secondary;
        let small = theme.metrics.small_text_size;
        let name = accounts::name(account);
        let state = match account.state {
            LockState::Unlocked => "Unlocked",
            LockState::Locked => "Locked",
            LockState::SignedOut => "Signed out",
        };
        let id = account.id.profile().to_string();
        let signed_out = account.state == LockState::SignedOut;
        let (shown, removed) = (account.id.clone(), account.clone());
        let header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .min_h(px(54.))
            .px(px(13.))
            .py(px(8.))
            .child(avatar(&name, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(relative(1.35))
                    .child(div().truncate().child(name.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_size(small)
                            .text_color(dim)
                            .child(format!("{} · {state}", server_name(&account.server))),
                    ),
            )
            .children(signed_out.then(|| {
                button(ElementId::Name(format!("sign-in-{id}").into()))
                    .label("Sign In…")
                    .size(px(26.))
                    .accessibility_label(format!("Sign in to {name}"))
                    .on_click(move |_, _, cx| {
                        let id = shown.clone();
                        window::with_main_window(cx, move |main, window, cx| {
                            main.show_account(id, window, cx)
                        });
                    })
            }))
            .child(
                button(ElementId::Name(format!("sign-out-{id}").into()))
                    .label("Sign Out…")
                    .size(px(26.))
                    .disabled(accounts::signing_out(&account.id, cx))
                    .accessibility_label(format!("Sign out of {name}"))
                    .on_click(move |_, window, cx| {
                        accounts::sign_out(&removed, window, cx, |error, window, cx| {
                            alerts::toast(backend::describe(&error), window, cx);
                        });
                    }),
            );
        let touch_id = self
            .store
            .read(cx)
            .get(&account.id)
            .and_then(|settings| settings.touch_id)
            .filter(|_| !signed_out);
        let mut card = card().row(header);
        if let Some(on) = touch_id {
            let (store, account) = (self.store.clone(), account.id.clone());
            card = card.row(setting_row(
                "Unlock with Touch ID",
                None,
                switch(
                    ElementId::Name(format!("touch-id-{id}").into()),
                    on,
                    "Unlock with Touch ID",
                )
                .on_change(move |on, _, cx| {
                    store.update(cx, |store, cx| {
                        store.update(&account, |settings| settings.touch_id = Some(on), cx)
                    });
                }),
                cx,
            ));
        }
        card.into_any_element()
    }
}

/// How people know the server: its host name.
fn server_name(server: &Server) -> String {
    match server {
        Server::BitwardenCom => "bitwarden.com".to_string(),
        Server::BitwardenEu => "bitwarden.eu".to_string(),
        Server::SelfHosted(url) => {
            let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
            rest.split('/').next().unwrap_or(rest).to_string()
        }
    }
}

fn add_account(cx: &mut App) {
    window::with_main_window(cx, |main, window, cx| main.add_account(window, cx));
}

impl Render for AccountsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accounts = Accounts::global(cx).read(cx).list().to_vec();
        let cards: Vec<AnyElement> = accounts
            .iter()
            .map(|account| self.render_account(account, cx))
            .collect();
        div().flex().flex_col().gap(px(12.)).children(cards).child(
            div().flex().child(
                button("add-account")
                    .label("Add Account…")
                    .icon(IconName::Plus)
                    .style(ButtonStyle::Normal)
                    .on_click(|_, _, cx| add_account(cx)),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_by_their_host() {
        assert_eq!(server_name(&Server::BitwardenEu), "bitwarden.eu");
        assert_eq!(
            server_name(&Server::SelfHosted("https://vault.example.com/".into())),
            "vault.example.com"
        );
        assert_eq!(
            server_name(&Server::SelfHosted("http://127.0.0.1:8080".into())),
            "127.0.0.1:8080"
        );
    }
}
