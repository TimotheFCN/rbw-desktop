//! The command line tool's link to the app's own `rbw`.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::process::in_an_app;
use crate::{CommandLineTool, Error, Result, ToolState};

/// Runs a shell script as an administrator, asking the person first;
/// `Ok(false)` when they cancel.
pub type Elevate<'a> = &'a dyn Fn(&str) -> Result<bool>;

/// A link named `rbw` in a folder, to the app's `rbw`.
pub struct Link {
    path: PathBuf,
    /// The app's `rbw`, resolved, as the link's state compares it.
    target: PathBuf,
}

impl Link {
    pub fn new(folder: &Path, target: &Path) -> Self {
        Self {
            path: folder.join("rbw"),
            target: std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf()),
        }
    }

    pub fn status(&self) -> CommandLineTool {
        let folder = self.path.parent().unwrap_or(Path::new("/"));
        CommandLineTool {
            path: self.path.clone(),
            state: self.state(),
            // /etc/paths puts /usr/local/bin on every terminal's PATH,
            // though not on the one launchd gives the app.
            on_path: cfg!(target_os = "macos")
                || in_path(folder, std::env::var_os("PATH").as_deref()),
        }
    }

    fn state(&self) -> ToolState {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return ToolState::Off;
        };
        if !metadata.file_type().is_symlink() {
            return ToolState::Taken;
        }
        match std::fs::canonicalize(&self.path) {
            Ok(resolved) if resolved == self.target => ToolState::On,
            // another copy of the app, such as the one this one replaced
            Ok(resolved) if in_an_app(&resolved) => ToolState::Off,
            Ok(_) => ToolState::Taken,
            // leads nowhere, such as where the app was before it moved
            Err(_) => ToolState::Off,
        }
    }

    /// Puts the link in place, replacing one that leads where the app used
    /// to be but never another program. A folder only an administrator can
    /// change goes through `elevate`; the person may cancel, which leaves
    /// the tool off.
    pub fn turn_on(&self, elevate: Option<Elevate>) -> Result<()> {
        match self.state() {
            ToolState::On => return Ok(()),
            ToolState::Taken => {
                return Err(Error::Other(format!(
                    "{} is another program",
                    self.path.display()
                )));
            }
            ToolState::Off => {}
        }
        match (self.link(), elevate) {
            (Err(error), Some(elevate)) if error.kind() == ErrorKind::PermissionDenied => {
                elevate(&self.link_script()).map(drop)
            }
            (result, _) => result.map_err(|error| {
                Error::Other(format!("can't make {}: {error}", self.path.display()))
            }),
        }
    }

    /// Takes the link away if it is the app's; anything else stays.
    pub fn turn_off(&self, elevate: Option<Elevate>) -> Result<()> {
        if self.state() != ToolState::On {
            return Ok(());
        }
        match (std::fs::remove_file(&self.path), elevate) {
            (Err(error), Some(elevate)) if error.kind() == ErrorKind::PermissionDenied => {
                elevate(&self.unlink_script()).map(drop)
            }
            (Err(error), _) if error.kind() != ErrorKind::NotFound => Err(Error::Other(format!(
                "can't remove {}: {error}",
                self.path.display()
            ))),
            _ => Ok(()),
        }
    }

    fn link(&self) -> std::io::Result<()> {
        if let Some(folder) = self.path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        if std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            std::fs::remove_file(&self.path)?;
        }
        std::os::unix::fs::symlink(&self.target, &self.path)
    }

    /// What `link` does, for a shell run as an administrator: `ln` without
    /// `-f`, so that a file that appeared meanwhile stays.
    fn link_script(&self) -> String {
        let folder = self.path.parent().unwrap_or(Path::new("/"));
        format!(
            "mkdir -p {} && {} && ln -s {} {}",
            shell_word(folder),
            self.unlink_script(),
            shell_word(&self.target),
            shell_word(&self.path),
        )
    }

    /// Removes the link if it is one, never a file.
    fn unlink_script(&self) -> String {
        let path = shell_word(&self.path);
        format!("{{ [ ! -L {path} ] || rm {path}; }}")
    }
}

/// Asks for an administrator with the system's prompt and runs `script`
/// as one. The script goes to the shell as an argument, so that nothing in
/// it needs quoting for AppleScript.
#[cfg(target_os = "macos")]
pub fn as_administrator(script: &str) -> Result<bool> {
    let prompt = format!(
        "{} wants to change the rbw command line tool.",
        crate::APP_NAME
    );
    let output = std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "do shell script (item 1 of argv) with prompt (item 2 of argv) \
             with administrator privileges",
            "-e",
            "end run",
            script,
            &prompt,
        ])
        .output()
        .map_err(|error| Error::Other(format!("can't run osascript: {error}")))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    match output.status.success() {
        true => Ok(true),
        // userCanceledErr
        false if stderr.contains("(-128)") => Ok(false),
        false => Err(Error::Other(format!("osascript: {}", stderr.trim()))),
    }
}

/// `path` as one word for a POSIX shell: in single quotes, in which
/// nothing expands, with its own single quotes closed and reopened around
/// them.
fn shell_word(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// Whether `folder` is one of the folders of a `PATH`.
fn in_path(folder: &Path, path: Option<&OsStr>) -> bool {
    let resolved = std::fs::canonicalize(folder).ok();
    path.is_some_and(|path| {
        std::env::split_paths(path).any(|dir| {
            dir == folder || (resolved.is_some() && std::fs::canonicalize(&dir).ok() == resolved)
        })
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs::Permissions;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use super::*;

    /// A folder of its own for a test, removed after it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rbw-desktop-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(std::fs::canonicalize(dir).unwrap())
        }

        /// A file at `path` under the folder, with its folders.
        fn file(&self, path: &str) -> PathBuf {
            let file = self.0.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "").unwrap();
            file
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // a test may have left a folder read-only
            let _ = std::fs::set_permissions(self.0.join("bin"), Permissions::from_mode(0o755));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn app_rbw(scratch: &Scratch, app: &str) -> PathBuf {
        scratch.file(&format!("{app}/libexec/rbw-desktop/rbw"))
    }

    #[test]
    fn turns_on_and_off_in_a_missing_folder() {
        let scratch = Scratch::new("link-on-off");
        let own = app_rbw(&scratch, "app");
        let link = Link::new(&scratch.0.join("bin"), &own);
        assert_eq!(link.status().state, ToolState::Off);

        link.turn_on(None).unwrap();
        assert_eq!(link.status().state, ToolState::On);
        assert_eq!(std::fs::read_link(scratch.0.join("bin/rbw")).unwrap(), own);
        link.turn_on(None).unwrap();

        link.turn_off(None).unwrap();
        assert_eq!(link.status().state, ToolState::Off);
        assert!(!scratch.0.join("bin/rbw").exists());
        link.turn_off(None).unwrap();
    }

    #[test]
    fn a_link_to_where_the_app_was_is_replaced() {
        let scratch = Scratch::new("link-stale");
        let own = app_rbw(&scratch, "app");
        let link = Link::new(&scratch.0.join("bin"), &own);
        std::fs::create_dir(scratch.0.join("bin")).unwrap();
        // the app before it moved
        symlink(scratch.0.join("gone/libexec/rbw-desktop/rbw"), &link.path).unwrap();
        assert_eq!(link.status().state, ToolState::Off);
        link.turn_on(None).unwrap();
        assert_eq!(link.status().state, ToolState::On);

        // another copy of the app, still there
        let copy = app_rbw(&scratch, "copy");
        std::fs::remove_file(&link.path).unwrap();
        symlink(&copy, &link.path).unwrap();
        assert_eq!(link.status().state, ToolState::Off);
        link.turn_off(None).unwrap();
        assert!(
            link.path.is_file(),
            "turning off leaves another copy's link"
        );
        link.turn_on(None).unwrap();
        assert_eq!(std::fs::read_link(&link.path).unwrap(), own);
    }

    #[test]
    fn another_program_is_left_alone() {
        let scratch = Scratch::new("link-taken");
        let own = app_rbw(&scratch, "app");
        let link = Link::new(&scratch.0.join("bin"), &own);

        let file = scratch.file("bin/rbw");
        assert_eq!(link.status().state, ToolState::Taken);
        assert!(link.turn_on(None).is_err());
        link.turn_off(None).unwrap();
        assert!(file.is_file() && !file.is_symlink());

        std::fs::remove_file(&file).unwrap();
        let homebrew = scratch.file("Cellar/rbw-ng/1.18.1/bin/rbw");
        symlink(&homebrew, &file).unwrap();
        assert_eq!(link.status().state, ToolState::Taken);
        assert!(link.turn_on(None).is_err());
        link.turn_off(None).unwrap();
        assert_eq!(std::fs::read_link(&file).unwrap(), homebrew);
    }

    #[test]
    fn a_folder_it_may_not_change_goes_to_an_administrator() {
        let scratch = Scratch::new("link-denied");
        let own = app_rbw(&scratch, "app");
        let bin = scratch.0.join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::set_permissions(&bin, Permissions::from_mode(0o555)).unwrap();
        let link = Link::new(&bin, &own);
        if std::fs::write(bin.join("probe"), "").is_ok() {
            // root ignores permissions: nothing to test
            return;
        }

        let scripts = RefCell::new(vec![]);
        let canceled = |script: &str| {
            scripts.borrow_mut().push(script.to_string());
            Ok(false)
        };
        link.turn_on(Some(&canceled)).unwrap();
        assert_eq!(link.status().state, ToolState::Off);
        let bin = bin.display();
        let own = own.display();
        assert_eq!(
            scripts.borrow().as_slice(),
            [format!(
                "mkdir -p '{bin}' && {{ [ ! -L '{bin}/rbw' ] || rm '{bin}/rbw'; }} && \
                 ln -s '{own}' '{bin}/rbw'"
            )]
        );
        let error = link.turn_on(None).unwrap_err();
        assert!(matches!(error, Error::Other(message) if message.contains("ermission")));
    }

    #[test]
    fn paths_are_quoted_for_the_shell() {
        assert_eq!(
            shell_word(Path::new("/Applications/A b")),
            "'/Applications/A b'"
        );
        assert_eq!(
            shell_word(Path::new("/tmp/it's $HOME")),
            r"'/tmp/it'\''s $HOME'"
        );
    }

    #[test]
    fn says_whether_a_folder_is_on_path() {
        let scratch = Scratch::new("in-path");
        let bin = scratch.0.join("bin");
        std::fs::create_dir(&bin).unwrap();
        symlink(&bin, scratch.0.join("alias")).unwrap();
        let path = |dirs: &[&str]| {
            std::env::join_paths(dirs.iter().map(|dir| scratch.0.join(dir))).unwrap()
        };
        assert!(in_path(&bin, Some(&path(&["other", "bin"]))));
        assert!(in_path(&bin, Some(&path(&["alias"]))));
        assert!(!in_path(&bin, Some(&path(&["other"]))));
        assert!(!in_path(&bin, None));
    }
}
