use std::path::PathBuf;

/// The command line tool: a link to the app's own `rbw` in a folder a
/// terminal looks in, so that the terminal runs the same copy as the app.
/// The two then share the accounts, the unlocked vaults and the SSH agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLineTool {
    /// Where the link goes: `/usr/local/bin/rbw` on macOS,
    /// `~/.local/bin/rbw` on Linux.
    pub path: PathBuf,
    pub state: ToolState,
    /// Whether a terminal looks in the link's folder. A Linux session puts
    /// `~/.local/bin` on `PATH` only if it existed when the person logged in.
    pub on_path: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    /// The link leads to the app's `rbw`.
    On,
    /// Nothing is there, or a link to where the app used to be, which
    /// turning the tool on replaces.
    Off,
    /// Another program is there, which the app leaves alone.
    Taken,
}
