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

/// The longest side of an icon the backend passes on. Icon services send 16
/// to 256 pixels; a decoder would allocate for whatever size a file claims.
const MAX_SIDE: u32 = 1024;

impl WebsiteIcon {
    /// The icon in what a server sent: a bitmap of at most [`MAX_SIDE`]
    /// pixels a side, by what its header says. `None` for anything else,
    /// SVG included: a server may pass on a website's SVG as it is, which
    /// can refer to other files.
    pub(crate) fn from_server(bytes: &[u8]) -> Option<Self> {
        let format = IconFormat::of(bytes)?;
        let sides = match format {
            IconFormat::Png => png_size(bytes).map(|size| vec![size]),
            IconFormat::Jpeg => jpeg_size(bytes).map(|size| vec![size]),
            IconFormat::Gif => gif_size(bytes).map(|size| vec![size]),
            IconFormat::Webp => webp_size(bytes).map(|size| vec![size]),
            IconFormat::Bmp => bmp_size(bytes.get(14..)?).map(|size| vec![size]),
            IconFormat::Ico => ico_sizes(bytes),
            IconFormat::Svg => None,
        }?;
        sides
            .iter()
            .all(|&(width, height)| width <= MAX_SIDE && height <= MAX_SIDE)
            .then(|| Self {
                format,
                bytes: bytes.into(),
            })
    }
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        bytes.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        bytes.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn u24_le(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

// The first chunk, IHDR, starts with the width and height.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    (bytes.get(12..16)? == b"IHDR").then_some(())?;
    Some((u32_be(bytes, 16)?, u32_be(bytes, 20)?))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((u16_le(bytes, 6)?, u16_le(bytes, 8)?))
}

// A BMP's information header, as in a BMP file after its 14-byte file
// header, or in an ICO.
fn bmp_size(header: &[u8]) -> Option<(u32, u32)> {
    match u32_le(header, 0)? {
        12 => Some((u16_le(header, 4)?, u16_le(header, 6)?)),
        // signed: a negative height means top-down rows
        _ => Some((
            u32_le(header, 4)?.cast_signed().unsigned_abs(),
            u32_le(header, 8)?.cast_signed().unsigned_abs(),
        )),
    }
}

// The size of the first frame, from its start-of-frame marker.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // fill bytes before a marker
            0xff => at += 1,
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                return Some((u16_be(bytes, at + 7)?, u16_be(bytes, at + 5)?));
            }
            // the image data begins without a frame
            0xd9 | 0xda => return None,
            0x01 | 0xd0..=0xd7 => at += 2,
            _ => at += 2 + usize::try_from(u16_be(bytes, at + 2)?).ok()?,
        }
    }
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            // a key frame's start code, then 14 bits of each side
            (bytes.get(23..26)? == [0x9d, 0x01, 0x2a]).then_some(())?;
            Some((u16_le(bytes, 26)? & 0x3fff, u16_le(bytes, 28)? & 0x3fff))
        }
        b"VP8L" => {
            (*bytes.get(20)? == 0x2f).then_some(())?;
            let bits = u32_le(bytes, 21)?;
            Some(((bits & 0x3fff) + 1, (bits >> 14 & 0x3fff) + 1))
        }
        b"VP8X" => Some((u24_le(bytes, 24)? + 1, u24_le(bytes, 27)? + 1)),
        _ => None,
    }
}

// Every image of an icon file: a decoder may pick any.
fn ico_sizes(bytes: &[u8]) -> Option<Vec<(u32, u32)>> {
    let count = usize::try_from(u16_le(bytes, 4)?).ok()?;
    if count == 0 {
        return None;
    }
    (0..count)
        .map(|index| {
            let entry = 6 + index * 16;
            let offset = usize::try_from(u32_le(bytes, entry + 12)?).ok()?;
            let image = bytes.get(offset..)?;
            if image.starts_with(b"\x89PNG") {
                png_size(image)
            } else {
                // stored as a BMP without its file header, twice as high
                // for its mask
                bmp_size(image).map(|(width, height)| (width, height / 2))
            }
        })
        .collect()
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
    // a final dot names the same host
    let host = host_port
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_lowercase();
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
            ("http://hidden.onion./", None),
            ("site.i2p..", None),
            ("https://Example.com./login", Some("example.com")),
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

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        bytes
    }

    fn ico(images: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0];
        bytes.extend(u16::try_from(images.len()).unwrap().to_le_bytes());
        let mut offset = 6 + 16 * images.len();
        for image in images {
            bytes.extend([0; 8]);
            bytes.extend(u32::try_from(image.len()).unwrap().to_le_bytes());
            bytes.extend(u32::try_from(offset).unwrap().to_le_bytes());
            offset += image.len();
        }
        for image in images {
            bytes.extend(image);
        }
        bytes
    }

    fn bmp_header(width: i32, height: i32) -> Vec<u8> {
        let mut header = 40_u32.to_le_bytes().to_vec();
        header.extend(width.to_le_bytes());
        header.extend(height.to_le_bytes());
        header.extend([1, 0, 32, 0]);
        header
    }

    #[test]
    fn icons_from_a_server_are_bitmaps_of_a_sensible_size() {
        let fits = |bytes: &[u8]| WebsiteIcon::from_server(bytes).is_some();
        assert!(fits(&png(32, 32)));
        assert!(fits(&png(1024, 1024)));
        assert!(!fits(&png(1025, 16)));
        assert!(!fits(&png(16, 100_000)));
        assert!(!fits(b"\x89PNG\r\n\x1a\n"));

        let mut bmp = b"BM".to_vec();
        bmp.extend([0; 12]);
        bmp.extend(bmp_header(48, -48));
        assert!(fits(&bmp));
        bmp.splice(18..22, 50_000_i32.to_le_bytes());
        assert!(!fits(&bmp));

        assert!(fits(&ico(&[png(16, 16), bmp_header(32, 64)])));
        assert!(!fits(&ico(&[png(16, 16), png(4096, 4096)])));
        assert!(!fits(&ico(&[png(16, 16), bmp_header(32, 100_000)])));
        assert!(!fits(&ico(&[])));

        assert!(fits(b"GIF89a\x10\0\x10\0"));
        assert!(!fits(b"GIF89a\xff\xff\x10\0"));

        // a JPEG with an APP0 segment, then its frame
        let jpeg = |width: u16, height: u16| {
            let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8];
            bytes.extend(height.to_be_bytes());
            bytes.extend(width.to_be_bytes());
            bytes.extend([1, 1, 0x11, 0]);
            bytes
        };
        assert!(fits(&jpeg(64, 64)));
        assert!(!fits(&jpeg(64, 2000)));

        let webp = |chunk: &[u8], data: &[u8]| {
            let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
            bytes.extend(chunk);
            bytes.extend([0; 4]);
            bytes.extend(data);
            bytes
        };
        let mut vp8x = vec![0; 4];
        vp8x.extend([63, 0, 0, 63, 0, 0]);
        assert!(fits(&webp(b"VP8X", &vp8x)));
        vp8x[4..7].copy_from_slice(&[0xff, 0xff, 0]);
        assert!(!fits(&webp(b"VP8X", &vp8x)));
        let vp8l = |width: u32, height: u32| {
            let mut data = vec![0x2f];
            data.extend(((width - 1) | (height - 1) << 14).to_le_bytes());
            data
        };
        assert!(fits(&webp(b"VP8L", &vp8l(32, 32))));
        assert!(!fits(&webp(b"VP8L", &vp8l(16_000, 32))));
        let mut vp8 = vec![0, 0, 0, 0x9d, 0x01, 0x2a];
        vp8.extend(16_u16.to_le_bytes());
        vp8.extend(16_u16.to_le_bytes());
        assert!(fits(&webp(b"VP8 ", &vp8)));

        assert!(!fits(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#));
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
