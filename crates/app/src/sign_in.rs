//! Signing in: adding an account (its server and email, then the master
//! password, then a code if the server asks for one), or signing in again
//! to an account that is signed out. Every step goes back, and an account
//! added for a sign-in that is then abandoned is removed again.

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    FontWeight, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    SharedString, Styled as _, Task, Window, actions, div, point, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{Account, AccountId, Error, LoginCode, Secret, Server, TwoStepMethod};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Opening, app_icon, button, field_error, motion,
    segmented_control, text_field,
};

use crate::accounts::{self, AccountMenu, Request};
use crate::backend;
use crate::window::plain_screen;

actions!(sign_in, [Back]);

const CONTEXT: &str = "SignIn";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Back, Some(CONTEXT))]);
}

const SERVERS: [&str; 3] = ["bitwarden.com", "bitwarden.eu", "Self-hosted"];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// The server and the email.
    Account,
    Password,
    /// A two-step login code, by one of the methods the account offers.
    Verification(Vec<TwoStepMethod>),
    /// The code the server emailed to verify this computer.
    NewDevice,
    /// The account's personal API key, which some servers ask for before
    /// they take the master password.
    ApiKey,
}

pub struct SignIn {
    /// The signed-out account signed in to, or `None` while adding one.
    existing: Option<Account>,
    accounts: Vec<Account>,
    /// Whether this is the first account on the computer.
    first: bool,
    step: Step,
    /// Names the step shown, so that each new one plays its entrance.
    generation: usize,
    /// Whether the last move was back, which slides the step in from the
    /// other side.
    went_back: bool,
    server: usize,
    server_url: Entity<InputState>,
    email: Entity<InputState>,
    password: Entity<InputState>,
    code: Entity<InputState>,
    api_key_id: Entity<InputState>,
    api_key_secret: Entity<InputState>,
    method: usize,
    email_sent: bool,
    /// The account made for this sign-in, removed again if it is abandoned.
    added: Option<AccountId>,
    problem: Option<&'static str>,
    /// Counts wrong entries, each of which shakes the field.
    wrong: usize,
    busy: bool,
    menu_open: bool,
    menu_focus: FocusHandle,
    task: Task<()>,
}

impl EventEmitter<Request> for SignIn {}

impl SignIn {
    /// Adds an account: the first on this computer when `accounts` is empty.
    pub fn add(accounts: Vec<Account>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = Self::new(None, accounts, window, cx);
        this.focus_step(window, cx);
        // An account added and then abandoned is removed, even when the
        // window closes in the middle.
        cx.on_release(|this, cx| {
            if let Some(id) = this.added.take() {
                backend::forget(cx, move |backend| backend.remove_account(&id));
            }
        })
        .detach();
        this
    }

    /// Signs in to an account that exists but is signed out.
    pub fn existing(
        account: Account,
        accounts: Vec<Account>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(Some(account), accounts, window, cx);
        this.step = Step::Password;
        this.focus_step(window, cx);
        this
    }

    fn new(
        existing: Option<Account>,
        accounts: Vec<Account>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = |placeholder: &'static str, masked: bool| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(masked)
            });
            cx.subscribe_in(&input, window, Self::on_input).detach();
            input
        };
        Self {
            first: accounts.is_empty(),
            existing,
            accounts,
            step: Step::Account,
            generation: 0,
            went_back: false,
            server: 0,
            server_url: input("https://vault.example.com", false),
            email: input("Email address", false),
            password: input("Master password", true),
            code: input("Verification code", false),
            api_key_id: input("client_id", false),
            api_key_secret: input("client_secret", true),
            method: 0,
            email_sent: false,
            added: None,
            problem: None,
            wrong: 0,
            busy: false,
            menu_open: false,
            menu_focus: cx.focus_handle(),
            task: Task::ready(()),
        }
    }

    /// The account signed in to, once there is one.
    pub fn account(&self) -> Option<&AccountId> {
        self.existing.as_ref().map(|account| &account.id)
    }

    pub fn set_accounts(&mut self, accounts: Vec<Account>, cx: &mut Context<Self>) {
        if accounts != self.accounts {
            self.accounts = accounts;
            cx.notify();
        }
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.next(window, cx),
            InputEvent::Change => {
                if self.problem.take().is_some() {
                    cx.notify();
                }
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn text(&self, input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    fn email(&self, cx: &App) -> String {
        match &self.existing {
            Some(account) => accounts::name(account),
            None => self.text(&self.email, cx),
        }
    }

    fn chosen_server(&self, cx: &App) -> Option<Server> {
        match self.server {
            0 => Some(Server::BitwardenCom),
            1 => Some(Server::BitwardenEu),
            _ => {
                let url = self.text(&self.server_url, cx);
                let url = url.trim_end_matches('/');
                let host = url.split_once("://").map_or(url, |(_, host)| host);
                if host.is_empty() || host.contains(char::is_whitespace) {
                    return None;
                }
                Some(Server::SelfHosted(if url.contains("://") {
                    url.to_string()
                } else {
                    format!("https://{url}")
                }))
            }
        }
    }

    fn go(&mut self, step: Step, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.step = step;
        self.went_back = back;
        self.generation += 1;
        self.problem = None;
        self.busy = false;
        self.focus_step(window, cx);
        cx.notify();
    }

    fn focus_step(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = match self.step {
            Step::Account => &self.email,
            Step::Password => &self.password,
            Step::Verification(_) | Step::NewDevice => &self.code,
            Step::ApiKey => &self.api_key_id,
        };
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match self.step {
            Step::Account if !self.first => cx.emit(Request::CancelAdd),
            Step::Account => {}
            Step::Password if self.existing.is_none() => {
                // The email or the server may change, so the account made
                // for them goes.
                if let Some(id) = self.added.take() {
                    backend::forget(cx, move |backend| backend.remove_account(&id));
                }
                self.go(Step::Account, true, window, cx);
            }
            Step::Password => {}
            Step::Verification(_) | Step::NewDevice | Step::ApiKey => {
                self.code
                    .update(cx, |code, cx| code.set_value("", window, cx));
                self.email_sent = false;
                self.go(Step::Password, true, window, cx);
            }
        }
    }

    fn can_go_back(&self) -> bool {
        match self.step {
            Step::Account => !self.first,
            Step::Password => self.existing.is_none(),
            Step::Verification(_) | Step::NewDevice | Step::ApiKey => true,
        }
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match self.step.clone() {
            Step::Account => self.check_account(window, cx),
            Step::Password => self.sign_in(None, None, window, cx),
            Step::Verification(methods) => {
                let code = self.text(&self.code, cx);
                if !code.is_empty() {
                    let method = methods[self.method.min(methods.len() - 1)];
                    let code = LoginCode::TwoStep { method, code };
                    self.sign_in(Some(code), None, window, cx);
                }
            }
            Step::NewDevice => {
                let code = self.text(&self.code, cx);
                if !code.is_empty() {
                    self.sign_in(Some(LoginCode::NewDevice(code)), None, window, cx);
                }
            }
            Step::ApiKey => {
                let id = self.text(&self.api_key_id, cx);
                let secret = Secret::new(self.text(&self.api_key_secret, cx));
                if !id.is_empty() && !secret.is_empty() {
                    self.sign_in(None, Some((id, secret)), window, cx);
                }
            }
        }
    }

    fn check_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chosen_server(cx).is_none() {
            self.problem = Some("Enter the address of your server.");
            self.server_url
                .update(cx, |input, cx| input.focus(window, cx));
        } else if !valid_email(&self.text(&self.email, cx)) {
            self.problem = Some("Enter your email address.");
            self.email.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.go(Step::Password, false, window, cx);
            return;
        }
        cx.notify();
    }

    /// Signs in with the master password and `code`, after registering this
    /// computer with `api_key` when there is one.
    fn sign_in(
        &mut self,
        code: Option<LoginCode>,
        api_key: Option<(String, Secret)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let password = Secret::new(self.password.read(cx).value().to_string());
        if password.is_empty() {
            return;
        }
        let account = self.account().or(self.added.as_ref()).cloned();
        let new = match account {
            Some(_) => None,
            None => self
                .chosen_server(cx)
                .map(|server| (server, self.email(cx))),
        };
        self.busy = true;
        self.problem = None;
        cx.notify();
        self.task = backend::request_in(
            window,
            cx,
            move |backend| {
                let id = match (account, new) {
                    (Some(id), _) => id,
                    (None, Some((server, email))) => backend.add_account(&server, &email)?,
                    (None, None) => return Err(Error::AccountNotFound),
                };
                let registered = match &api_key {
                    Some((key_id, secret)) => backend.register_device(&id, key_id, secret),
                    None => Ok(()),
                };
                let signed_in =
                    registered.and_then(|()| backend.sign_in(&id, &password, code.as_ref()));
                Ok((id, signed_in))
            },
            |this, result, window, cx| {
                let signed_in = match result {
                    Ok((id, signed_in)) => {
                        if this.existing.is_none() {
                            this.added = Some(id.clone());
                        }
                        signed_in.map(|()| id)
                    }
                    Err(error) => Err(error),
                };
                match signed_in {
                    Ok(id) => {
                        // Done: the account stays, and the spinner keeps
                        // going until its vault replaces this screen.
                        this.added = None;
                        cx.emit(Request::SignedIn(id));
                    }
                    Err(error) => this.fail(error, window, cx),
                }
                cx.notify();
            },
        );
    }

    fn fail(&mut self, error: Error, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = false;
        match error {
            Error::TwoStepRequired { mut methods } => {
                // The authenticator app first: it needs nothing sent.
                methods.sort_by_key(|method| match method {
                    TwoStepMethod::Authenticator => 0,
                    TwoStepMethod::YubiKey => 1,
                    TwoStepMethod::Email => 2,
                });
                self.method = 0;
                self.go(Step::Verification(methods), false, window, cx);
            }
            Error::RegistrationRequired => self.go(Step::ApiKey, false, window, cx),
            Error::NewDeviceCodeRequired => {
                self.code
                    .update(cx, |code, cx| code.set_value("", window, cx));
                self.go(Step::NewDevice, false, window, cx);
            }
            error => {
                self.problem = Some(match error {
                    Error::AccountExists(_) => "This account is already on this computer.",
                    Error::TwoStepUnsupported => {
                        "This account's two-step login method can't be used here."
                    }
                    ref error => backend::describe(error),
                });
                if matches!(
                    error,
                    Error::WrongPassword | Error::WrongCode | Error::WrongApiKey
                ) {
                    self.wrong += 1;
                    let input = match error {
                        Error::WrongCode => &self.code,
                        Error::WrongApiKey => &self.api_key_secret,
                        _ => &self.password,
                    };
                    input.update(cx, |input, cx| {
                        input.focus(window, cx);
                        input.select_all(window, cx);
                    });
                }
            }
        }
    }

    fn send_email_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.account().or(self.added.as_ref()).cloned() else {
            return;
        };
        let password = Secret::new(self.password.read(cx).value().to_string());
        self.busy = true;
        cx.notify();
        self.task = backend::request_in(
            window,
            cx,
            move |backend| backend.send_email_code(&id, &password),
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.email_sent = true;
                        this.code.update(cx, |code, cx| code.focus(window, cx));
                    }
                    Err(error) => this.problem = Some(backend::describe(&error)),
                }
                cx.notify();
            },
        );
    }

    fn render_title(&mut self, cx: &mut Context<Self>) -> (SharedString, Option<AnyElement>) {
        let email = self.email(cx);
        // A sentence takes the column's width, so that a long one wraps.
        let line = |text: String| Some(div().flex_1().min_w_0().child(text).into_any_element());
        match &self.step {
            Step::Account if self.first => (
                "Welcome".into(),
                line("Sign in to your Bitwarden or Vaultwarden account.".into()),
            ),
            Step::Account => (
                "Add an Account".into(),
                line("Sign in to another Bitwarden account.".into()),
            ),
            Step::Password if self.existing.is_some() => {
                ("Sign In".into(), Some(self.render_who(email, cx)))
            }
            Step::Password => ("Master Password".into(), line(email)),
            Step::Verification(methods) => {
                let method = methods.get(self.method).copied();
                (
                    "Two-Step Login".into(),
                    line(match method {
                        Some(TwoStepMethod::Email) => format!("Enter the code sent to {email}."),
                        Some(TwoStepMethod::YubiKey) => "Insert your YubiKey and touch it.".into(),
                        _ => "Enter the code from your authenticator app.".into(),
                    }),
                )
            }
            Step::ApiKey => (
                "Personal API Key".into(),
                line(
                    "Your server asks for this account's API key. Find it in the web \
                     vault, in Settings › Security › Keys."
                        .into(),
                ),
            ),
            Step::NewDevice => (
                "Verify This Computer".into(),
                line(format!("Enter the code sent to {email}.")),
            ),
        }
    }

    /// The account's name, which opens the account menu when there are
    /// other accounts to go to.
    fn render_who(&mut self, name: String, cx: &mut Context<Self>) -> AnyElement {
        let Some(account) = self.existing.clone().filter(|_| self.accounts.len() > 1) else {
            return div().child(name).into_any_element();
        };
        div()
            .relative()
            .child(
                button("account")
                    .label(name.clone())
                    .trailing_icon(IconName::ChevronsUpDown)
                    .style(ButtonStyle::Flat)
                    .size(px(24.))
                    .accessibility_label(format!("{name}, switch account"))
                    .on_click(
                        cx.listener(|this, _, window, cx| accounts::toggle(this, window, cx)),
                    ),
            )
            .child(accounts::menu(
                self,
                &self.accounts,
                &account.id,
                Opening::BelowCentered,
                cx,
            ))
            .into_any_element()
    }

    fn render_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let shake = motion::shake("sign-in-field", self.wrong, window, cx);
        let shaking =
            |field: AnyElement| div().relative().left(shake).child(field).into_any_element();
        match self.step.clone() {
            Step::Account => {
                let this = cx.entity().downgrade();
                let mut fields = vec![
                    segmented_control("server", SERVERS, self.server)
                        .on_select(move |index, window, cx| {
                            this.update(cx, |this, cx| {
                                this.server = index;
                                this.problem = None;
                                let input = if index == 2 {
                                    &this.server_url
                                } else {
                                    &this.email
                                };
                                input.update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            })
                            .ok();
                        })
                        .into_any_element(),
                ];
                if self.server == 2 {
                    fields.push(
                        motion::screen_enter(
                            "server-url",
                            div().child(text_field(
                                "server-url",
                                &self.server_url,
                                "Server address",
                            )),
                            point(px(0.), px(-6.)),
                            cx,
                        )
                        .into_any_element(),
                    );
                }
                fields.push(text_field("email", &self.email, "Email address").into_any_element());
                fields
            }
            Step::Password => vec![shaking(
                text_field("password", &self.password, "Master password").into_any_element(),
            )],
            Step::Verification(methods) => {
                let mut fields = vec![];
                if methods.len() > 1 {
                    let this = cx.entity().downgrade();
                    fields.push(
                        segmented_control(
                            "method",
                            methods.iter().map(|method| match method {
                                TwoStepMethod::Authenticator => "Authenticator app",
                                TwoStepMethod::Email => "Email",
                                TwoStepMethod::YubiKey => "YubiKey",
                            }),
                            self.method,
                        )
                        .on_select(move |index, window, cx| {
                            this.update(cx, |this, cx| {
                                this.method = index;
                                this.problem = None;
                                this.code.update(cx, |code, cx| code.focus(window, cx));
                                cx.notify();
                            })
                            .ok();
                        })
                        .into_any_element(),
                    );
                }
                fields.push(shaking(
                    text_field("code", &self.code, "Verification code").into_any_element(),
                ));
                if methods.get(self.method) == Some(&TwoStepMethod::Email) {
                    let dim = cx.theme().colors.text_secondary;
                    fields.push(if self.email_sent {
                        div()
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(dim)
                            .child("Code sent. Check your email.")
                            .into_any_element()
                    } else {
                        div()
                            .flex()
                            .justify_center()
                            .child(
                                button("email-code")
                                    .label("Email Me a Code")
                                    .style(ButtonStyle::Link)
                                    .size(px(24.))
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_email_code(window, cx)
                                    })),
                            )
                            .into_any_element()
                    });
                }
                fields
            }
            Step::ApiKey => vec![
                text_field("api-key-id", &self.api_key_id, "API key client_id").into_any_element(),
                shaking(
                    text_field(
                        "api-key-secret",
                        &self.api_key_secret,
                        "API key client_secret",
                    )
                    .into_any_element(),
                ),
            ],
            Step::NewDevice => vec![shaking(
                text_field("code", &self.code, "Verification code").into_any_element(),
            )],
        }
    }
}

impl AccountMenu for SignIn {
    fn menu_open(&self) -> bool {
        self.menu_open
    }

    fn set_menu_open(&mut self, open: bool) {
        self.menu_open = open;
    }

    fn menu_focus(&self) -> &FocusHandle {
        &self.menu_focus
    }

    fn menu_return(&self, cx: &App) -> FocusHandle {
        self.password.read(cx).focus_handle(cx)
    }
}

fn valid_email(email: &str) -> bool {
    email
        .split_once('@')
        .is_some_and(|(name, domain)| !name.is_empty() && !domain.is_empty())
        && !email.contains(char::is_whitespace)
}

impl Render for SignIn {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, subtitle) = self.render_title(cx);
        let fields = self.render_fields(window, cx);
        let theme = cx.theme();
        let dim = theme.colors.text_secondary;
        let action = match self.step {
            Step::Account => "Continue",
            Step::Password => "Sign In",
            Step::Verification(_) | Step::NewDevice => "Verify",
            Step::ApiKey => "Sign In",
        };
        // Equal halves, like an alert's buttons; alone, the action fills the
        // row.
        let buttons = div()
            .mt(px(4.))
            .flex()
            .gap(px(8.))
            .when(self.can_go_back(), |d| {
                d.child(
                    div().flex_1().child(
                        button("back")
                            .label("Back")
                            .wide()
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.back(&Back, window, cx)),
                            ),
                    ),
                )
            })
            .child(
                div().flex_1().child(
                    button("next")
                        .label(action)
                        .style(ButtonStyle::Primary)
                        .wide()
                        .busy(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
                ),
            );
        let slide = if self.went_back { px(-24.) } else { px(24.) };
        let step = motion::screen_enter(
            ("sign-in-step", self.generation),
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_center()
                        .mb(px(8.))
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::BOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .mt(px(3.))
                                .min_h(px(24.))
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(dim)
                                .children(subtitle),
                        ),
                )
                .children(fields)
                // The message has a line of its own, so that it moves
                // nothing when it appears.
                .child(
                    div()
                        .min_h(px(18.))
                        .flex()
                        .justify_center()
                        .text_center()
                        .children(self.problem.map(|problem| field_error(problem, cx))),
                )
                .child(buttons),
            point(slide, px(0.)),
            cx,
        );
        // The column keeps its height from step to step, so that the icon
        // and the title stay where they are.
        let content = div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::back))
            .w(px(320.))
            .h(px(340.))
            .flex()
            .flex_col()
            .items_center()
            .child(app_icon(cx))
            .child(div().mt(px(16.)).w_full().child(step));
        plain_screen("sign-in", content, window, cx)
    }
}
