//! Running `rbw`: one short process per call, for one profile, with
//! secrets on stdin and errors as JSON on stderr.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::{AccountId, Error, OutdatedRbw, REQUIRED_RBW, Result, TwoStepMethod};

/// Overrides where to find `rbw`: the path of the binary.
pub const RBW_ENV: &str = "RBW_DESKTOP_RBW";

/// How long a call may take: local calls answer in milliseconds, but the
/// first one may start the agent, and calls that reach the server wait for
/// it.
#[derive(Clone, Copy)]
pub enum Wait {
    Local,
    Network,
}

impl Wait {
    fn limit(self) -> Duration {
        match self {
            Self::Local => Duration::from_secs(30),
            Self::Network => Duration::from_secs(120),
        }
    }
}

/// Where the packages put the `rbw` and `rbw-agent` that come with the app,
/// from the folder above the one of its executable: the bundle's
/// `Contents/Helpers` on macOS, `libexec/rbw-desktop` beside `bin`
/// elsewhere. Neither is on `PATH`, so they never take the place of the
/// `rbw` a terminal runs.
const BUNDLED: &str = if cfg!(target_os = "macos") {
    "Helpers"
} else {
    "libexec/rbw-desktop"
};

pub struct Rbw {
    path: PathBuf,
    // the rbw-agent next to rbw, as the app's packages bundle both
    agent: Option<PathBuf>,
}

impl Rbw {
    /// Finds an `rbw` that is recent enough: the one in `RBW_DESKTOP_RBW`,
    /// else the one on `PATH`, which the command line runs too, else the
    /// one bundled with this program. With the last, it also returns the
    /// older one on `PATH`, which would restart the account's agent.
    pub fn find() -> Result<(Self, Option<OutdatedRbw>)> {
        if let Some(path) = std::env::var_os(RBW_ENV).filter(|path| !path.is_empty()) {
            return Ok((Self::checked(PathBuf::from(path))?, None));
        }
        let installed = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("rbw"))
                .find(|path| path.is_file())
        });
        // resolved: a link to the executable, such as one in a folder on
        // PATH, leads to where the app was unpacked
        let bundled = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .ok()
            .and_then(|exe| Some(exe.parent()?.parent()?.join(BUNDLED).join("rbw")))
            .filter(|path| path.is_file());
        choose(
            installed.map(Self::at),
            bundled.map(Self::at),
            Self::version,
        )
    }

    /// The `rbw` at this path, if it is recent enough.
    pub fn checked(path: PathBuf) -> Result<Self> {
        let rbw = Self::at(path);
        let version = rbw.version()?;
        enough(rbw, version)
    }

    /// Whether the agent is somewhere that goes away: in an app on a disk
    /// image, or one that macOS runs from a temporary copy because it was
    /// opened where it was downloaded (App Translocation). A service set up
    /// with it would run nothing once the image is ejected or the Mac
    /// restarts.
    pub fn agent_is_transient(&self) -> bool {
        cfg!(target_os = "macos") && self.agent.as_deref().is_some_and(transient)
    }

    fn at(path: PathBuf) -> Self {
        let agent = Some(path.with_file_name("rbw-agent")).filter(|agent| agent.is_file());
        Self { path, agent }
    }

    /// The version `rbw --version` prints.
    pub fn version(&self) -> Result<Version> {
        let output = self.run(None, &["--version"], None, Wait::Local)?;
        Ok(Version::of(&String::from_utf8_lossy(&output)))
    }

    /// Runs `rbw` for a profile and returns its output, or how it failed.
    pub fn run<S: AsRef<OsStr>>(
        &self,
        profile: Option<&AccountId>,
        args: &[S],
        stdin: Option<&[u8]>,
        wait: Wait,
    ) -> Result<Zeroizing<Vec<u8>>, Failure> {
        self.run_with_env(profile, args, stdin, &[], wait)
    }

    /// Runs `rbw` with more variables in its environment, for secrets that
    /// it reads from there.
    pub fn run_with_env<S: AsRef<OsStr>>(
        &self,
        profile: Option<&AccountId>,
        args: &[S],
        stdin: Option<&[u8]>,
        env: &[(&str, &str)],
        wait: Wait,
    ) -> Result<Zeroizing<Vec<u8>>, Failure> {
        self.run_within(profile, args, stdin, env, wait.limit())
    }

    fn run_within<S: AsRef<OsStr>>(
        &self,
        profile: Option<&AccountId>,
        args: &[S],
        stdin: Option<&[u8]>,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> Result<Zeroizing<Vec<u8>>, Failure> {
        let mut command = Command::new(&self.path);
        command
            .args(args)
            .envs(env.iter().copied())
            .env("RBW_NONINTERACTIVE", "1")
            .env("RBW_ERROR_FORMAT", "json")
            .env_remove("RBW_SESSION")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match profile.filter(|profile| !profile.is_default()) {
            Some(profile) => command.env("RBW_PROFILE", profile.profile()),
            None => command.env_remove("RBW_PROFILE"),
        };
        if let Some(agent) = &self.agent {
            command.env("RBW_AGENT", agent);
        }
        // from the start: a stalled rbw may not even read its input
        let deadline = Instant::now() + limit;
        let mut child = command.spawn().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Error::RbwNotFound,
            _ => Error::Other(format!("can't run {}: {error}", self.path.display())),
        })?;
        let (input_pipe, output_pipe, error_pipe) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take());

        // the scope waits for its threads, which end once rbw has exited or
        // been killed and its pipes have closed
        std::thread::scope(|scope| {
            let stdout = scope.spawn(|| read_all(output_pipe));
            let stderr = scope.spawn(|| read_all(error_pipe));
            if let (Some(input), Some(mut pipe)) = (stdin, input_pipe) {
                // a failed write shows as rbw's own error about its input
                scope.spawn(move || pipe.write_all(input));
            }

            let mut pause = Duration::from_millis(1);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(Error::Timeout.into());
                    }
                    Ok(None) => {
                        std::thread::sleep(pause);
                        pause = (pause * 2).min(Duration::from_millis(20));
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(Error::Other(format!("can't wait for rbw: {error}")).into());
                    }
                }
            };
            let stdout = stdout.join().unwrap_or_default();
            let stderr = stderr.join().unwrap_or_default();
            if status.success() {
                Ok(stdout)
            } else {
                Err(failure(&String::from_utf8_lossy(&stderr)))
            }
        })
    }

    /// Runs `rbw` and parses its JSON output.
    pub fn json<T: serde::de::DeserializeOwned, S: AsRef<OsStr>>(
        &self,
        profile: Option<&AccountId>,
        args: &[S],
        stdin: Option<&[u8]>,
        wait: Wait,
    ) -> Result<T> {
        let output = self.run(profile, args, stdin, wait)?;
        serde_json::from_slice(&output)
            .map_err(|error| Error::Other(format!("rbw answered with unexpected JSON: {error}")))
    }
}

fn read_all(pipe: Option<impl std::io::Read>) -> Zeroizing<Vec<u8>> {
    // room for an item with its secrets, so that the buffer rarely grows
    // and leaves copies behind
    let mut output = Zeroizing::new(Vec::with_capacity(64 * 1024));
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut output);
    }
    output
}

// The rbw on PATH when it is recent enough, else the bundled one, with the
// older one on PATH.
fn choose(
    installed: Option<Rbw>,
    bundled: Option<Rbw>,
    version: impl Fn(&Rbw) -> Result<Version>,
) -> Result<(Rbw, Option<OutdatedRbw>)> {
    let Some(installed) = installed else {
        let bundled = bundled.ok_or(Error::RbwNotFound)?;
        let version = version(&bundled)?;
        return Ok((enough(bundled, version)?, None));
    };
    let outdated = match version(&installed) {
        Ok(found) if found.is_enough() => return Ok((installed, None)),
        Ok(found) => Ok(OutdatedRbw {
            path: installed.path,
            version: found.text,
        }),
        Err(error) => Err(error),
    };
    match (bundled, outdated) {
        // an rbw that can't say its version can't run an agent either
        (Some(bundled), outdated) => {
            let found = version(&bundled)?;
            Ok((enough(bundled, found)?, outdated.ok()))
        }
        (None, Ok(outdated)) => Err(Error::RbwTooOld {
            found: outdated.version,
            required: REQUIRED_RBW.to_string(),
        }),
        (None, Err(error)) => Err(error),
    }
}

fn enough(rbw: Rbw, version: Version) -> Result<Rbw> {
    if version.is_enough() {
        Ok(rbw)
    } else {
        Err(Error::RbwTooOld {
            found: version.text,
            required: REQUIRED_RBW.to_string(),
        })
    }
}

fn transient(path: &std::path::Path) -> bool {
    path.starts_with("/Volumes")
        || path
            .components()
            .any(|part| part.as_os_str() == "AppTranslocation")
}

/// An `rbw --version`.
pub struct Version {
    /// rbw-ng's version number, or the whole text for another program, such
    /// as the original rbw.
    pub text: String,
    // None for another program than rbw-ng
    numbers: Option<[u64; 3]>,
}

impl Version {
    fn of(output: &str) -> Self {
        let line = output.trim();
        let numbers = line
            .strip_prefix("rbw-ng ")
            .and_then(|version| numbers(version.trim()));
        Self {
            text: match numbers {
                Some(_) => line["rbw-ng ".len()..].trim().to_string(),
                None => line.to_string(),
            },
            numbers,
        }
    }

    fn is_enough(&self) -> bool {
        self.numbers
            .is_some_and(|numbers| Some(numbers) >= self::numbers(REQUIRED_RBW))
    }
}

// The major, minor and patch numbers, without a pre-release or build part.
fn numbers(version: &str) -> Option<[u64; 3]> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse().ok());
    let numbers = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(numbers)
}

/// How a call failed: the error, and rbw's code for it when it gave one.
#[derive(Debug)]
pub struct Failure {
    pub code: Option<String>,
    pub error: Error,
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self { code: None, error }
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        failure.error
    }
}

#[derive(serde::Deserialize)]
struct JsonError {
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    methods: Vec<u8>,
}

/// How a call failed, from the JSON line `RBW_ERROR_FORMAT=json` prints.
fn failure(stderr: &str) -> Failure {
    let Some(error) = stderr
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<JsonError>(line).ok())
    else {
        return Error::Other(stderr.trim().to_string()).into();
    };
    let code = error.code.clone();
    Failure {
        error: error_from(error),
        code: Some(code),
    }
}

fn error_from(error: JsonError) -> Error {
    match error.code.as_str() {
        "not_logged_in" => Error::NotSignedIn,
        "locked" | "session_invalid" => Error::Locked,
        "input_required" => Error::PasswordRequired,
        "incorrect_password" => Error::WrongPassword,
        "two_step_required" => Error::TwoStepRequired {
            methods: error
                .methods
                .iter()
                .filter_map(|id| match id {
                    0 => Some(TwoStepMethod::Authenticator),
                    1 => Some(TwoStepMethod::Email),
                    3 => Some(TwoStepMethod::YubiKey),
                    _ => None,
                })
                .collect(),
        },
        "new_device_code_required" => Error::NewDeviceCodeRequired,
        "registration_required" => Error::RegistrationRequired,
        "incorrect_code" => Error::WrongCode,
        "two_step_unsupported" => Error::TwoStepUnsupported,
        "reprompt_required" => Error::RepromptRequired,
        "touch_id_canceled" => Error::TouchIdCanceled,
        "touch_id_unavailable" => Error::TouchIdUnavailable,
        "not_found" => Error::NotFound,
        "conflict" => Error::Conflict,
        "forbidden" => Error::ReadOnly,
        "invalid_input" => Error::InvalidItem(error.message),
        "network" => Error::Offline,
        "wrong_server" => Error::WrongServer,
        "server" => Error::Server(error.message),
        _ => Error::Other(error.message),
    }
}

pub fn method_id(method: TwoStepMethod) -> u8 {
    match method {
        TwoStepMethod::Authenticator => 0,
        TwoStepMethod::Email => 1,
        TwoStepMethod::YubiKey => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_from(stderr: &str) -> Error {
        failure(stderr).error
    }

    #[test]
    fn errors_come_from_their_code() {
        assert_eq!(
            error_from(r#"{"code":"locked","message":"rbw item list: the vault is locked"}"#),
            Error::Locked
        );
        assert_eq!(
            error_from(
                "warning first\n{\"code\":\"two_step_required\",\"methods\":[0,1,2],\"message\":\"x\"}\n"
            ),
            Error::TwoStepRequired {
                methods: vec![TwoStepMethod::Authenticator, TwoStepMethod::Email]
            }
        );
        assert_eq!(
            error_from(r#"{"code":"server","status":500,"message":"rbw sync: 500"}"#),
            Error::Server("rbw sync: 500".into())
        );
        assert_eq!(
            error_from(r#"{"code":"registration_required","message":"m"}"#),
            Error::RegistrationRequired
        );
        assert_eq!(
            error_from(r#"{"code":"reprompt_required","message":"m"}"#),
            Error::RepromptRequired
        );
        assert_eq!(
            error_from(r#"{"code":"wrong_server","message":"m"}"#),
            Error::WrongServer
        );
        assert_eq!(
            error_from(r#"{"code":"something_new","message":"m"}"#),
            Error::Other("m".into())
        );
        assert_eq!(
            error_from("error: unexpected argument\n"),
            Error::Other("error: unexpected argument".into())
        );
    }

    #[test]
    fn versions_count_for_rbw_ng_only() {
        let enough = |output: &str| Version::of(output).is_enough();
        assert!(enough("rbw-ng 1.19.0\n"));
        assert!(enough("rbw-ng 1.20.3"));
        assert!(enough("rbw-ng 2.0.0-dev"));
        assert!(!enough("rbw-ng 1.18.1\n"));
        assert!(!enough("rbw 1.19.0"));
        assert!(!enough("rbw 1.13.2"));
        assert!(!enough("rbw-ng 1.19"));
        assert!(!enough(""));
        assert_eq!(Version::of("rbw-ng 1.18.1\n").text, "1.18.1");
        assert_eq!(Version::of("rbw 1.13.2\n").text, "rbw 1.13.2");
    }

    #[test]
    fn prefers_the_rbw_on_path_and_says_when_it_is_too_old() {
        let installed = || Some(Rbw::at(PathBuf::from("/usr/bin/rbw")));
        let bundled = || Some(Rbw::at(PathBuf::from("/usr/libexec/rbw-desktop/rbw")));
        let versions = |on_path: &'static str, next_to_app: &'static str| {
            move |rbw: &Rbw| match rbw.path.starts_with("/usr/bin") {
                true if on_path.is_empty() => Err(Error::Other("broken".into())),
                true => Ok(Version::of(on_path)),
                false => Ok(Version::of(next_to_app)),
            }
        };
        let chosen = |installed, bundled, on_path, next_to_app| {
            choose(installed, bundled, versions(on_path, next_to_app))
                .map(|(rbw, outdated)| (rbw.path, outdated.map(|rbw| rbw.version)))
        };
        let usr = PathBuf::from("/usr/bin/rbw");
        let libexec = PathBuf::from("/usr/libexec/rbw-desktop/rbw");

        assert_eq!(
            chosen(installed(), bundled(), "rbw-ng 1.19.0", "rbw-ng 1.20.0"),
            Ok((usr.clone(), None))
        );
        assert_eq!(
            chosen(installed(), bundled(), "rbw-ng 1.18.1", "rbw-ng 1.19.0"),
            Ok((libexec.clone(), Some("1.18.1".to_string())))
        );
        assert_eq!(
            chosen(installed(), bundled(), "rbw 1.13.2", "rbw-ng 1.19.0"),
            Ok((libexec.clone(), Some("rbw 1.13.2".to_string())))
        );
        assert_eq!(
            chosen(installed(), bundled(), "", "rbw-ng 1.19.0"),
            Ok((libexec.clone(), None))
        );
        assert_eq!(
            chosen(None, bundled(), "", "rbw-ng 1.19.0"),
            Ok((libexec, None))
        );
        assert_eq!(
            chosen(installed(), None, "rbw-ng 1.18.1", ""),
            Err(Error::RbwTooOld {
                found: "1.18.1".into(),
                required: REQUIRED_RBW.into()
            })
        );
        assert_eq!(
            chosen(None, bundled(), "", "rbw-ng 1.18.1"),
            Err(Error::RbwTooOld {
                found: "1.18.1".into(),
                required: REQUIRED_RBW.into()
            })
        );
        assert_eq!(chosen(None, None, "", ""), Err(Error::RbwNotFound));
    }

    #[test]
    fn an_agent_on_a_disk_image_or_translocated_is_transient() {
        let app = "Fastwarden.app/Contents/Helpers/rbw-agent";
        assert!(transient(&PathBuf::from("/Volumes/Fastwarden").join(app)));
        assert!(transient(
            &PathBuf::from("/private/var/folders/x/T/AppTranslocation/1A2B/d").join(app)
        ));
        assert!(!transient(&PathBuf::from("/Applications").join(app)));
        assert!(!transient(&PathBuf::from("/opt/homebrew/bin/rbw-agent")));
    }

    #[test]
    fn runs_with_a_timeout_and_reports_a_missing_rbw() {
        let rbw = Rbw::at(PathBuf::from("/nonexistent/rbw"));
        assert_eq!(
            rbw.run(None, &["status"], None, Wait::Local)
                .unwrap_err()
                .error,
            Error::RbwNotFound
        );
        assert_eq!(
            failure(r#"{"code":"two_step_code_sent","message":"m"}"#)
                .code
                .as_deref(),
            Some("two_step_code_sent")
        );
        let sleep = Rbw::at(PathBuf::from("/bin/sleep"));
        let start = Instant::now();
        let output = sleep.run(None, &["0.05"], None, Wait::Local).unwrap();
        assert!(output.is_empty());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn times_out_while_rbw_doesnt_read_its_input() {
        // more than a pipe holds, to a program that never reads it
        let input = vec![b'x'; 1024 * 1024];
        let sleep = Rbw::at(PathBuf::from("/bin/sleep"));
        let start = Instant::now();
        let failure = sleep
            .run_within(None, &["10"], Some(&input), &[], Duration::from_millis(200))
            .unwrap_err();
        assert_eq!(failure.error, Error::Timeout);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
