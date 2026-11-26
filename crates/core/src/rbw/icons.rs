//! Websites' icons kept on disk, so that each one is asked of the server
//! once a month at most. A cache only: failing to write it changes nothing
//! but how often the server is asked. Each account has a folder of its own:
//! another server may have an icon that this one hasn't, and removing an
//! account forgets which sites it had.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use super::private_dir;
use crate::{AccountId, WebsiteIcon};

// As long as Vaultwarden keeps the icons it fetched, and the sites it found
// none for.
const ICON_LIFE: Duration = Duration::from_secs(30 * 24 * 3600);
const MISS_LIFE: Duration = Duration::from_secs(3 * 24 * 3600);
// A few thousand icons; past it, the oldest go until a quarter is free.
const SIZE_LIMIT: u64 = 20 * 1024 * 1024;
// Looking for what to prune reads every file's size, so it waits for this
// many writes, or this many bytes written, since it last looked; the first
// write of a run looks at once.
const PRUNE_WRITES: usize = 50;
const PRUNE_BYTES: usize = 1024 * 1024;

pub struct IconCache {
    dir: PathBuf,
    /// Writes and bytes written since the cache was last pruned.
    writes: AtomicUsize,
    written: AtomicUsize,
    pruned: AtomicBool,
}

impl IconCache {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            writes: AtomicUsize::new(0),
            written: AtomicUsize::new(0),
            pruned: AtomicBool::new(false),
        }
    }

    /// Counts a write of `len` bytes, and says whether it is time to prune.
    fn prune_due(&self, len: usize) -> bool {
        let writes = self.writes.fetch_add(1, Ordering::Relaxed) + 1;
        let written = self
            .written
            .fetch_add(len, Ordering::Relaxed)
            .saturating_add(len);
        let first = !self.pruned.swap(true, Ordering::Relaxed);
        if first || writes >= PRUNE_WRITES || written >= PRUNE_BYTES {
            self.writes.store(0, Ordering::Relaxed);
            self.written.store(0, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    // "profile-" keeps a profile called ".." from naming another folder
    fn folder(&self, account: &AccountId) -> Option<PathBuf> {
        let name = if account.is_default() {
            "default".to_string()
        } else {
            format!("profile-{}", account.profile())
        };
        (!name.contains('/')).then(|| self.dir.join(name))
    }

    /// The host's icon, `Some(None)` if the account's server had none, or
    /// `None` if the server must be asked.
    pub fn get(&self, account: &AccountId, host: &str) -> Option<Option<WebsiteIcon>> {
        let dir = self.folder(account)?;
        let icon = dir.join(format!("{host}.icon"));
        if fresh(&icon, ICON_LIFE) {
            let bytes = std::fs::read(&icon).ok()?;
            return Some(Some(WebsiteIcon::from_server(&bytes)?));
        }
        fresh(&dir.join(format!("{host}.miss")), MISS_LIFE).then_some(None)
    }

    pub fn put(&self, account: &AccountId, host: &str, icon: Option<&WebsiteIcon>) {
        let Some(dir) = self.folder(account) else {
            return;
        };
        let (keep, drop) = match icon {
            Some(_) => ("icon", "miss"),
            None => ("miss", "icon"),
        };
        let bytes = icon.map_or(&[][..], |icon| &icon.bytes);
        // which sites the vault has is for this user alone
        if private_dir(&self.dir).is_err() || private_dir(&dir).is_err() {
            return;
        }
        // written whole, then moved into place, as another call may read or
        // write the same host meanwhile
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let partial = dir.join(format!(
            "{host}.{}-{}.partial",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        if write_private(&partial, bytes).is_err()
            || std::fs::rename(&partial, dir.join(format!("{host}.{keep}"))).is_err()
        {
            let _ = std::fs::remove_file(&partial);
            return;
        }
        let _ = std::fs::remove_file(dir.join(format!("{host}.{drop}")));
        if self.prune_due(bytes.len()) {
            self.prune();
        }
    }

    /// Prunes each account's folder on its own.
    pub fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                prune(&entry.path());
            }
        }
    }

    /// Forgets every icon.
    pub fn clear(&self) -> std::io::Result<()> {
        remove(&self.dir)
    }

    /// Forgets the account's icons.
    pub fn forget(&self, account: &AccountId) -> std::io::Result<()> {
        self.folder(account).map_or(Ok(()), |dir| remove(&dir))
    }
}

/// A new file only this user can read, with the bytes.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

fn remove(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

// One account's folder.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
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
        let account = AccountId::default();
        assert_eq!(cache.get(&account, "github.com"), None);
        let icon = WebsiteIcon {
            format: IconFormat::Png,
            bytes: b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x10\0\0\0\x10"
                .as_slice()
                .into(),
        };
        cache.put(&account, "github.com", Some(&icon));
        assert_eq!(cache.get(&account, "github.com"), Some(Some(icon.clone())));
        cache.put(&account, "github.com", None);
        assert_eq!(cache.get(&account, "github.com"), Some(None));
        cache.put(&account, "github.com", Some(&icon));
        assert_eq!(cache.get(&account, "github.com"), Some(Some(icon.clone())));
        assert!(!dir.join("default/github.com.miss").exists());

        // another account's server may have icons this one hasn't
        let other = AccountId::new("work");
        assert_eq!(cache.get(&other, "github.com"), None);
        cache.put(&other, "example.com", None);
        assert_eq!(cache.get(&account, "example.com"), None);
        cache.forget(&other).unwrap();
        assert_eq!(cache.get(&other, "example.com"), None);
        assert_eq!(cache.get(&account, "github.com"), Some(Some(icon)));
        let parent = AccountId::new("..");
        cache.put(&parent, "example.com", None);
        assert!(dir.join("profile-../example.com.miss").exists());
        assert!(cache.folder(&AccountId::new("a/b")).is_none());
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join("default")), 0o700);
            assert_eq!(mode(&dir.join("default/github.com.icon")), 0o600);
            assert_eq!(mode(&dir.join("profile-../example.com.miss")), 0o600);
        }
        cache.clear().unwrap();
        assert!(!dir.exists());
        cache.clear().unwrap();
    }

    #[test]
    fn prunes_the_oldest_past_the_limit() {
        let dir = std::env::temp_dir().join(format!("rbw-desktop-prune-{}", std::process::id()));
        let cache = IconCache::new(dir.clone());
        let account = dir.join("default");
        std::fs::create_dir_all(&account).unwrap();
        let old = account.join("old.example.icon");
        std::fs::write(&old, vec![0; (SIZE_LIMIT / 2) as usize]).unwrap();
        let file = std::fs::File::options().write(true).open(&old).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        std::fs::write(
            account.join("new.example.icon"),
            vec![0; (SIZE_LIMIT / 2 + 1) as usize],
        )
        .unwrap();
        // the first write of a run prunes
        cache.put(&AccountId::default(), "tiny.example", None);
        assert!(!old.exists());
        assert!(account.join("new.example.icon").exists());
        cache.clear().unwrap();
    }

    #[test]
    fn prunes_every_so_many_writes_or_bytes() {
        let cache = IconCache::new(PathBuf::from("/nonexistent"));
        assert!(cache.prune_due(10));
        for _ in 1..PRUNE_WRITES {
            assert!(!cache.prune_due(10));
        }
        assert!(cache.prune_due(10));
        assert!(!cache.prune_due(PRUNE_BYTES - 1));
        assert!(cache.prune_due(1));
        assert!(!cache.prune_due(0));
    }
}
