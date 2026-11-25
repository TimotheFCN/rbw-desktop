//! The command line tool: how the app names where it is.

use std::path::Path;

/// A path as people read it, with `~` for their home folder.
pub fn shown(path: &Path) -> String {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_in_the_home_folder_start_with_a_tilde() {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return;
        };
        let home = Path::new(&home);
        assert_eq!(shown(&home.join(".local/bin/rbw")), "~/.local/bin/rbw");
        assert_eq!(shown(Path::new("/usr/local/bin/rbw")), "/usr/local/bin/rbw");
    }
}
