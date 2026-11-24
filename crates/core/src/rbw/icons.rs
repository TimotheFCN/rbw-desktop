//! Websites' icons kept on disk, so that each one is asked of the server
//! once a month at most. A cache only: failing to write it changes nothing
//! but how often the server is asked.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::WebsiteIcon;

// As long as Vaultwarden keeps the icons it fetched, and the sites it found
// none for.
const ICON_LIFE: Duration = Duration::from_secs(30 * 24 * 3600);
const MISS_LIFE: Duration = Duration::from_secs(3 * 24 * 3600);
// A few thousand icons; past it, the oldest go until a quarter is free.
const SIZE_LIMIT: u64 = 20 * 1024 * 1024;

pub struct IconCache {
    dir: PathBuf,
}

impl IconCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The host's icon, `Some(None)` if the server had none, or `None` if
    /// the server must be asked.
    pub fn get(&self, host: &str) -> Option<Option<WebsiteIcon>> {
        let icon = self.dir.join(format!("{host}.icon"));
        if fresh(&icon, ICON_LIFE) {
            let bytes = std::fs::read(&icon).ok()?;
            return Some(Some(WebsiteIcon::from_server(&bytes)?));
        }
        fresh(&self.dir.join(format!("{host}.miss")), MISS_LIFE).then_some(None)
    }

    pub fn put(&self, host: &str, icon: Option<&WebsiteIcon>) {
        let (keep, drop) = match icon {
            Some(_) => ("icon", "miss"),
            None => ("miss", "icon"),
        };
        let bytes = icon.map_or(&[][..], |icon| &icon.bytes);
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        // written whole, then moved into place, as another call may read or
        // write the same host meanwhile
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let partial = self.dir.join(format!(
            "{host}.{}-{}.partial",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        if std::fs::write(&partial, bytes).is_err()
            || std::fs::rename(&partial, self.dir.join(format!("{host}.{keep}"))).is_err()
        {
            let _ = std::fs::remove_file(&partial);
            return;
        }
        let _ = std::fs::remove_file(self.dir.join(format!("{host}.{drop}")));
        self.prune();
    }

    /// Forgets every icon.
    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<_> = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let metadata = entry.metadata().ok()?;
                Some((metadata.modified().ok()?, metadata.len(), entry.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
        if total <= SIZE_LIMIT {
            return;
        }
        files.sort();
        for (_, len, path) in files {
            if total <= SIZE_LIMIT / 4 * 3 {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= len;
            }
        }
    }
}

fn fresh(path: &Path, life: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .is_ok_and(|age| age < life)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IconFormat;

    #[test]
    fn keeps_icons_and_misses_until_told_to_forget() {
        let dir = std::env::temp_dir().join(format!("rbw-desktop-icons-{}", std::process::id()));
        let cache = IconCache::new(dir.clone());
        assert_eq!(cache.get("github.com"), None);
        let icon = WebsiteIcon {
            format: IconFormat::Png,
            bytes: b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x10\0\0\0\x10"
                .as_slice()
                .into(),
        };
        cache.put("github.com", Some(&icon));
        assert_eq!(cache.get("github.com"), Some(Some(icon.clone())));
        cache.put("github.com", None);
        assert_eq!(cache.get("github.com"), Some(None));
        cache.put("github.com", Some(&icon));
        assert_eq!(cache.get("github.com"), Some(Some(icon)));
        assert!(!dir.join("github.com.miss").exists());
        cache.clear().unwrap();
        assert!(!dir.exists());
        cache.clear().unwrap();
    }

    #[test]
    fn prunes_the_oldest_past_the_limit() {
        let dir = std::env::temp_dir().join(format!("rbw-desktop-prune-{}", std::process::id()));
        let cache = IconCache::new(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.example.icon");
        std::fs::write(&old, vec![0; (SIZE_LIMIT / 2) as usize]).unwrap();
        let file = std::fs::File::options().write(true).open(&old).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        std::fs::write(
            dir.join("new.example.icon"),
            vec![0; (SIZE_LIMIT / 2 + 1) as usize],
        )
        .unwrap();
        cache.put("tiny.example", None);
        assert!(!old.exists());
        assert!(dir.join("new.example.icon").exists());
        cache.clear().unwrap();
    }
}
