//! Websites' icons, shown in place of a login's letter tile when the
//! preference allows.
//!
//! An icon rbw keeps on disk is read off the main thread, a batch at a
//! time: for the first rows of a list before it shows (`read_cached` with
//! the items), for the rows about to scroll in ([`Icons::prefetch`]) and
//! for any row drawn. One never kept is asked of the server only while a
//! row that shows it is drawn, a few at a time on threads of their own: the
//! first time, rbw asks the server, which can take seconds, and that must
//! never hold up what people wait for. Until an icon is known its tile is
//! neutral; when one arrives, the views observing [`Icons`] redraw and it
//! fades in. A website known to have none shows its letter. Icons are
//! decoded small, off the main thread, and the most recently shown are
//! kept. Icons are kept by host: the logins of one website share one read,
//! one image and one request to the server.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Context, Entity, Global, RenderImage, Task};
use image::imageops::FilterType;
use rbw_desktop_core::{
    AccountId, Backend, IconFormat, ItemKind, ItemSummary, WebsiteIcon, host_from_uri,
};
use rbw_desktop_ui::TileIcon;

use crate::backend;

/// The largest an icon is kept, in pixels: the item header's tile at twice
/// the size of a point.
const PIXELS: u32 = 128;
/// How many icons are asked of the server at once.
const AT_ONCE: usize = 3;
/// How many icons are kept; the ones shown longest ago go first.
const KEPT: usize = 400;
/// How many websites' hosts are remembered before they are all forgotten.
const HOSTS_KEPT: usize = 20_000;
/// How long after a failure, such as being offline, an icon is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long a website known to have no icon shows its letter before rbw
/// is asked again, which answers from its own record of it for days.
const MISSING_FOR: Duration = Duration::from_secs(3600);

/// The host whose icon a website shows, as rbw names it: every login of
/// the host shares one icon, whatever the rest of its address.
type Host = Arc<str>;

enum Slot {
    /// Being read from rbw's cache. `letter` keeps the letter on screen
    /// meanwhile, for a website that had none.
    Reading {
        letter: bool,
    },
    /// Not in rbw's cache: asked of the server when a row shows it.
    Uncached,
    /// Asked of the server; dropping the task drops the answer.
    Fetching {
        letter: bool,
        _answer: Task<()>,
    },
    Ready {
        image: Arc<RenderImage>,
        /// When it was shown last, in [`Icons::tick`]s.
        shown: u64,
    },
    Missing(Instant),
    Failed(Instant),
}

/// What a slot puts in a tile, to know whether a change is to be drawn.
#[derive(PartialEq)]
enum Shows {
    Pending,
    Letter,
    Image(*const RenderImage),
}

impl Slot {
    fn shows(&self) -> Shows {
        match self {
            Slot::Ready { image, .. } => Shows::Image(Arc::as_ptr(image)),
            Slot::Reading { letter } | Slot::Fetching { letter, .. } if *letter => Shows::Letter,
            Slot::Reading { .. } | Slot::Fetching { .. } | Slot::Uncached => Shows::Pending,
            Slot::Missing(_) | Slot::Failed(_) => Shows::Letter,
        }
    }
}

/// What rbw's cache holds for a host: an icon, `Some(None)` for none to
/// show, and `None` when only the server can say.
type CachedIcon = Option<Option<Arc<RenderImage>>>;

/// Icons read from rbw's cache, to put in [`Icons`] with
/// [`Icons::insert_cached`].
pub struct Cached(Vec<(AccountId, Host, CachedIcon)>);

pub struct Icons {
    enabled: bool,
    /// Counts each time icons are turned on or off, so that an answer asked
    /// for before goes nowhere.
    generation: u64,
    /// Each account's icons, by host.
    icons: HashMap<AccountId, HashMap<Host, Slot>>,
    /// The host of each website address met, `None` for one that gets no
    /// icon, so that drawing a row finds it without making it again.
    hosts: HashMap<String, Option<Host>>,
    /// How many of them are ready, to know without counting when some
    /// must go.
    ready: usize,
    fetching: usize,
    /// Counts the icons shown, to know which were shown longest ago.
    tick: u64,
    /// Icons to read from rbw's cache once the current update is over,
    /// together.
    to_read: Vec<(AccountId, Host)>,
}

struct Shared(Entity<Icons>);

impl Global for Shared {}

/// Shows no icons until the preferences allow them ([`crate::preferences`]).
pub fn init(cx: &mut App) {
    let icons = cx.new(|_| Icons {
        enabled: false,
        generation: 0,
        icons: HashMap::new(),
        hosts: HashMap::new(),
        ready: 0,
        fetching: 0,
        tick: 0,
        to_read: vec![],
    });
    cx.set_global(Shared(icons));
}

/// The website an item's icon is kept under, if it is a login with one.
fn website(item: &ItemSummary) -> Option<&str> {
    (item.kind == ItemKind::Login)
        .then(|| item.websites.first().map(String::as_str))
        .flatten()
}

impl Icons {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<Shared>().0.clone()
    }

    /// Shows icons or stops; stopping forgets them.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.generation += 1;
        if !enabled {
            for slot in std::mem::take(&mut self.icons)
                .into_values()
                .flat_map(HashMap::into_values)
            {
                if let Slot::Ready { image, .. } = slot {
                    cx.drop_image(image, None);
                }
            }
            // The answers on the way were dropped with their slots.
            self.hosts.clear();
            self.ready = 0;
            self.fetching = 0;
            self.to_read.clear();
        }
        cx.notify();
    }

    /// The host of a website address, remembered.
    fn host(&mut self, website: &str) -> Option<Host> {
        if let Some(host) = self.hosts.get(website) {
            return host.clone();
        }
        if self.hosts.len() >= HOSTS_KEPT {
            self.hosts.clear();
        }
        let host: Option<Host> = host_from_uri(website).map(Host::from);
        self.hosts.insert(website.to_string(), host.clone());
        host
    }

    fn slot_mut(&mut self, account: &AccountId, host: &str) -> Option<&mut Slot> {
        self.icons.get_mut(account)?.get_mut(host)
    }

    /// What the item's tile shows. Call it while drawing the item: an icon
    /// not known yet is read from rbw's cache, or asked of the server if
    /// it isn't there and not too many are on the way already.
    pub fn icon(
        &mut self,
        account: &AccountId,
        item: &ItemSummary,
        cx: &mut Context<Self>,
    ) -> TileIcon {
        if !self.enabled {
            return TileIcon::Letter;
        }
        let Some(host) = website(item).and_then(|website| self.host(website)) else {
            return TileIcon::Letter;
        };
        self.tick += 1;
        let tick = self.tick;
        let letter = match self.slot_mut(account, &host) {
            Some(Slot::Ready { image, shown }) => {
                *shown = tick;
                return TileIcon::Image(image.clone());
            }
            Some(Slot::Reading { letter } | Slot::Fetching { letter, .. }) => {
                return if *letter {
                    TileIcon::Letter
                } else {
                    TileIcon::Pending
                };
            }
            Some(Slot::Missing(at)) if at.elapsed() < MISSING_FOR => return TileIcon::Letter,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => return TileIcon::Letter,
            Some(Slot::Missing(_)) => {
                self.read(account, host, true, cx);
                return TileIcon::Letter;
            }
            None => {
                self.read(account, host, false, cx);
                return TileIcon::Pending;
            }
            Some(Slot::Failed(_)) => true,
            Some(Slot::Uncached) => false,
        };
        if self.fetching < AT_ONCE {
            self.fetch(account, host, item, letter, cx);
        }
        if letter {
            TileIcon::Letter
        } else {
            TileIcon::Pending
        }
    }

    /// The icon rbw keeps for a website, read from its cache, never asked
    /// of the server: for an item being made, whose address may be half
    /// typed. `None` until it is known to be there.
    fn cached_only(
        &mut self,
        account: &AccountId,
        website: &str,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        if !self.enabled {
            return None;
        }
        let host = self.host(website)?;
        self.tick += 1;
        let tick = self.tick;
        match self.slot_mut(account, &host) {
            Some(Slot::Ready { image, shown }) => {
                *shown = tick;
                Some(image.clone())
            }
            Some(_) => None,
            None => {
                self.read(account, host, true, cx);
                None
            }
        }
    }

    /// Reads the icons of rows about to show from rbw's cache, so that
    /// they are there when the rows are drawn. Asks no server.
    pub fn prefetch<'a>(
        &mut self,
        account: &AccountId,
        items: impl IntoIterator<Item = &'a ItemSummary>,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        for item in items {
            if let Some(host) = website(item).and_then(|website| self.host(website))
                && self.slot_mut(account, &host).is_none()
            {
                self.read(account, host, false, cx);
            }
        }
    }

    /// Puts icons read from rbw's cache before a list showed in place. An
    /// icon already known, or being asked of the server, stays as it is.
    pub fn insert_cached(&mut self, cached: Cached, cx: &mut Context<Self>) {
        if !self.enabled || cached.0.is_empty() {
            return;
        }
        let mut changed = false;
        for (account, host, image) in cached.0 {
            if matches!(
                self.slot_mut(&account, &host),
                Some(Slot::Ready { .. } | Slot::Fetching { .. })
            ) {
                continue;
            }
            changed |= self.settle(account, host, image, cx);
        }
        changed |= self.forget_old(cx);
        if changed {
            cx.notify();
        }
    }

    /// Puts what rbw's cache said in the slot; whether that changes what
    /// shows.
    fn settle(
        &mut self,
        account: AccountId,
        host: Host,
        image: CachedIcon,
        cx: &mut Context<Self>,
    ) -> bool {
        let slot = match image {
            Some(Some(image)) => Slot::Ready {
                image,
                shown: self.tick,
            },
            Some(None) => Slot::Missing(Instant::now()),
            None => Slot::Uncached,
        };
        self.put(account, host, slot, cx)
    }

    /// Puts `slot` in place of what was there; whether that changes what
    /// shows, or calls for the server. A fetch it replaces no longer
    /// counts, and an image it replaces is let go.
    fn put(&mut self, account: AccountId, host: Host, slot: Slot, cx: &mut Context<Self>) -> bool {
        let shows = slot.shows();
        let asks = matches!(slot, Slot::Uncached);
        let ready = matches!(slot, Slot::Ready { .. });
        let old = self.icons.entry(account).or_default().insert(host, slot);
        let changed = old.as_ref().is_none_or(|old| old.shows() != shows) || asks;
        match old {
            Some(Slot::Ready { image, .. }) => {
                self.ready -= 1;
                if Shows::Image(Arc::as_ptr(&image)) != shows {
                    cx.drop_image(image, None);
                }
            }
            Some(Slot::Fetching { .. }) => self.fetching = self.fetching.saturating_sub(1),
            _ => {}
        }
        self.ready += usize::from(ready);
        changed
    }

    /// Reads the icon from rbw's cache with the others asked for in this
    /// update.
    fn read(&mut self, account: &AccountId, host: Host, letter: bool, cx: &mut Context<Self>) {
        self.put(account.clone(), host.clone(), Slot::Reading { letter }, cx);
        self.to_read.push((account.clone(), host));
        if self.to_read.len() > 1 {
            return;
        }
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let reading = this.update(cx, |this, cx| {
                let wanted = std::mem::take(&mut this.to_read);
                backend::call(cx, move |backend| Ok(read(backend, wanted)))
            });
            let Ok(reading) = reading else { return };
            let Ok(cached) = reading.await else { return };
            this.update(cx, |this, cx| {
                // Icons were turned off, and maybe on again, since: what
                // is asked now is asked again.
                if this.generation != generation {
                    return;
                }
                let mut changed = false;
                for (account, host, image) in cached.0 {
                    if matches!(this.slot_mut(&account, &host), Some(Slot::Reading { .. })) {
                        changed |= this.settle(account, host, image, cx);
                    }
                }
                changed |= this.forget_old(cx);
                if changed {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks rbw for the icon, which asks the server.
    fn fetch(
        &mut self,
        account: &AccountId,
        host: Host,
        item: &ItemSummary,
        letter: bool,
        cx: &mut Context<Self>,
    ) {
        let generation = self.generation;
        let (asked, item) = (account.clone(), item.clone());
        let (answer_account, answer_host) = (account.clone(), host.clone());
        let answer = backend::request_on_thread(
            "website icon",
            cx,
            move |backend| Ok(backend.website_icon(&asked, &item)?.and_then(decode)),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                let slot = match result {
                    Ok(Some(image)) => Slot::Ready {
                        image,
                        shown: this.tick,
                    },
                    Ok(None) => Slot::Missing(Instant::now()),
                    Err(error) => {
                        log::info!("a website icon: {error}");
                        Slot::Failed(Instant::now())
                    }
                };
                // Replacing the fetch's own slot counts it as done, which
                // lets the rows waiting for one ask when they draw again.
                this.put(answer_account, answer_host, slot, cx);
                this.forget_old(cx);
                cx.notify();
            },
        );
        self.put(
            account.clone(),
            host,
            Slot::Fetching {
                letter,
                _answer: answer,
            },
            cx,
        );
        self.fetching += 1;
    }

    /// Forgets the icons shown longest ago, beyond the ones kept; whether
    /// any went.
    fn forget_old(&mut self, cx: &mut Context<Self>) -> bool {
        if self.ready <= KEPT {
            return false;
        }
        let mut ready: Vec<(u64, AccountId, Host)> = self
            .icons
            .iter()
            .flat_map(|(account, hosts)| {
                hosts.iter().filter_map(move |(host, slot)| match slot {
                    Slot::Ready { shown, .. } => Some((*shown, account.clone(), host.clone())),
                    _ => None,
                })
            })
            .collect();
        ready.sort_unstable_by_key(|(shown, ..)| *shown);
        let mut gone = false;
        for (_, account, host) in &ready[..ready.len() - KEPT] {
            if let Some(hosts) = self.icons.get_mut(account)
                && let Some(Slot::Ready { image, .. }) = hosts.remove(host)
            {
                self.ready -= 1;
                cx.drop_image(image, None);
                gone = true;
            }
        }
        gone
    }
}

/// [`Icons::icon`] from anywhere.
pub fn icon(account: &AccountId, item: &ItemSummary, cx: &mut App) -> TileIcon {
    Icons::global(cx).update(cx, |icons, cx| icons.icon(account, item, cx))
}

/// The icon rbw keeps for a website, if it keeps one, for the item being
/// made in the editor: read from rbw's cache, never asked of the server.
pub fn kept_icon(account: &AccountId, website: &str, cx: &mut App) -> Option<Arc<RenderImage>> {
    Icons::global(cx).update(cx, |icons, cx| icons.cached_only(account, website, cx))
}

/// Reads the icons of `items` that rbw keeps on disk and decodes them, off
/// the main thread, for [`Icons::insert_cached`]: the icons of a list's
/// first rows, read with the list so that they show with it. Logins of
/// the same host are read once.
pub fn read_cached<'a>(
    backend: &dyn Backend,
    account: &AccountId,
    items: impl IntoIterator<Item = &'a ItemSummary>,
) -> Cached {
    let mut wanted: Vec<(AccountId, Host)> = vec![];
    for item in items {
        if let Some(host) = website(item).and_then(host_from_uri)
            && !wanted.iter().any(|(_, known)| **known == *host)
        {
            wanted.push((account.clone(), Host::from(host)));
        }
    }
    read(backend, wanted)
}

fn read(backend: &dyn Backend, wanted: Vec<(AccountId, Host)>) -> Cached {
    Cached(
        wanted
            .into_iter()
            .map(|(account, host)| {
                // A cache that can't be read is as good as empty.
                let cached = backend
                    .cached_website_icon_host(&account, &host)
                    .unwrap_or_default();
                // One that can't be decoded is as good as none.
                let image = cached.map(|icon| icon.and_then(decode));
                (account, host, image)
            })
            .collect(),
    )
}

/// The icon as an image to draw, at most [`PIXELS`] a side.
fn decode(icon: WebsiteIcon) -> Option<Arc<RenderImage>> {
    let format = match icon.format {
        IconFormat::Png => image::ImageFormat::Png,
        IconFormat::Jpeg => image::ImageFormat::Jpeg,
        IconFormat::Gif => image::ImageFormat::Gif,
        IconFormat::Webp => image::ImageFormat::WebP,
        IconFormat::Bmp => image::ImageFormat::Bmp,
        IconFormat::Ico => image::ImageFormat::Ico,
        // The backend passes no SVG on: one can refer to other files.
        IconFormat::Svg => return None,
    };
    let decoded = match image::load_from_memory_with_format(&icon.bytes, format) {
        Ok(decoded) => decoded,
        Err(error) => {
            log::info!("a website icon that can't be read: {error}");
            return None;
        }
    };
    let decoded = if decoded.width() > PIXELS || decoded.height() > PIXELS {
        decoded.resize(PIXELS, PIXELS, FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut pixels = decoded.into_rgba8();
    // GPUI draws BGRA.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])))
}

#[cfg(test)]
mod tests {
    use rbw_desktop_core::ItemId;

    use super::*;

    fn login(website: &str) -> ItemSummary {
        ItemSummary {
            id: ItemId("a".into()),
            kind: ItemKind::Login,
            name: "x".into(),
            subtitle: None,
            websites: vec![website.into()],
            folder: None,
            organization: None,
            favorite: false,
            has_code: false,
            reprompt: false,
            editable: true,
            modified: None,
        }
    }

    #[test]
    fn the_logins_of_a_host_share_its_icon() {
        let pages = [
            login("https://github.com/login"),
            login("https://GitHub.com/settings"),
        ];
        let hosts: Vec<_> = pages
            .iter()
            .map(|item| website(item).and_then(host_from_uri))
            .collect();
        assert_eq!(hosts[0].as_deref(), Some("github.com"));
        assert_eq!(hosts[0], hosts[1]);
        let mut note = login("https://example.com");
        note.kind = ItemKind::SecureNote;
        assert_eq!(website(&note), None);
    }
}
