use std::sync::Arc;

use crate::{ItemKind, ItemSummary};

/// A website's icon, as the image the server's icon service sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebsiteIcon {
    pub format: IconFormat,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Ico,
    Svg,
}

impl IconFormat {
    /// The format of an image, from its first bytes; `None` for anything
    /// else, such as a web page.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        Some(match bytes {
            [0x89, b'P', b'N', b'G', ..] => Self::Png,
            [0xff, 0xd8, 0xff, ..] => Self::Jpeg,
            [b'G', b'I', b'F', b'8', ..] => Self::Gif,
            [
                b'R',
                b'I',
                b'F',
                b'F',
                _,
                _,
                _,
                _,
                b'W',
                b'E',
                b'B',
                b'P',
                ..,
            ] => Self::Webp,
            [b'B', b'M', ..] => Self::Bmp,
            [0, 0, 1, 0, ..] => Self::Ico,
            _ => {
                let start = &bytes[..bytes.len().min(1024)];
                let text = String::from_utf8_lossy(start).to_ascii_lowercase();
                let text = text.trim_start_matches('\u{feff}').trim_start();
                if (text.starts_with("<svg") || text.starts_with("<?xml")) && text.contains("<svg")
                {
                    Self::Svg
                } else {
                    return None;
                }
            }
        })
    }
}

/// The host whose icon an item shows, chosen as the official clients do: a
/// login's first website, when it is a web address or a bare host name
/// with a dot, and never a `.onion` or `.i2p` site, whose names they keep
/// from the icon service.
pub(crate) fn icon_host(item: &ItemSummary) -> Option<String> {
    if item.kind != ItemKind::Login {
        return None;
    }
    let website = item.websites.first()?.trim();
    let rest = match website.split_once("://") {
        None if website.contains('.') => website,
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => rest,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = host_port.split(':').next()?.to_lowercase();
    let valid = host.contains('.')
        && !host.starts_with('.')
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !matches!(host.rsplit('.').next(), Some("onion" | "i2p"));
    valid.then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ItemId;

    fn login(website: &str) -> ItemSummary {
        ItemSummary {
            id: ItemId("a".into()),
            kind: ItemKind::Login,
            name: "x".into(),
            subtitle: None,
            websites: vec![website.into(), "https://second.example".into()],
            folder: None,
            organization: None,
            favorite: false,
            has_code: false,
            editable: true,
            reprompt: false,
        }
    }

    #[test]
    fn hosts_as_the_official_clients_pick_them() {
        for (website, host) in [
            ("https://GitHub.com/login", Some("github.com")),
            ("http://user:pw@example.org:8443/x?y#z", Some("example.org")),
            ("forum.example.net", Some("forum.example.net")),
            ("forum.example.net/path", Some("forum.example.net")),
            ("192.168.1.1", Some("192.168.1.1")),
            ("localhost", None),
            ("http://localhost:8080", None),
            ("ftp://files.example.com", None),
            ("androidapp://com.example.app", None),
            ("iosapp://com.example.app", None),
            ("http://hidden.onion", None),
            ("site.i2p", None),
            ("https://[::1]/", None),
            ("", None),
        ] {
            assert_eq!(icon_host(&login(website)).as_deref(), host, "{website}");
        }
        let mut card = login("https://bank.example");
        card.kind = ItemKind::Card;
        assert_eq!(icon_host(&card), None);
    }

    #[test]
    fn formats_from_the_first_bytes() {
        assert_eq!(IconFormat::of(b"\x89PNG\r\n\x1a\n"), Some(IconFormat::Png));
        assert_eq!(IconFormat::of(b"\0\0\x01\0\x01\0"), Some(IconFormat::Ico));
        assert_eq!(
            IconFormat::of(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(IconFormat::Webp)
        );
        assert_eq!(
            IconFormat::of(br#"<?xml version="1.0"?><svg xmlns="x"/>"#),
            Some(IconFormat::Svg)
        );
        assert_eq!(IconFormat::of(b"<!doctype html><html>"), None);
        assert_eq!(IconFormat::of(b""), None);
    }
}
