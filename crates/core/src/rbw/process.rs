//! Running `rbw`: one short process per call, for one profile, with
//! secrets on stdin and errors as JSON on stderr.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::{AccountId, Error, Result, TwoStepMethod};

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

pub struct Rbw {
    path: PathBuf,
    // the rbw-agent next to rbw, when rbw-desktop bundles both
    agent: Option<PathBuf>,
}

impl Rbw {
    /// Finds `rbw`: the path in `RBW_DESKTOP_RBW`, the one next to this
    /// program, then the one on `PATH`.
    pub fn find() -> Result<Self> {
        let path = std::env::var_os(RBW_ENV)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                let exe = std::env::current_exe().ok()?;
                Some(exe.with_file_name("rbw")).filter(|path| path.is_file())
            })
            .or_else(|| {
                std::env::split_paths(&std::env::var_os("PATH")?)
                    .map(|dir| dir.join("rbw"))
                    .find(|path| path.is_file())
            })
            .ok_or(Error::RbwNotFound)?;
        Ok(Self::at(path))
    }

    pub fn at(path: PathBuf) -> Self {
        let agent = Some(path.with_file_name("rbw-agent")).filter(|agent| agent.is_file());
        Self { path, agent }
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
        let mut child = command.spawn().map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Error::RbwNotFound,
            _ => Error::Other(format!("can't run {}: {error}", self.path.display())),
        })?;

        let stdout = read_in_background(child.stdout.take());
        let stderr = read_in_background(child.stderr.take());
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // a failed write shows as rbw's own error about its input
            let _ = pipe.write_all(input);
        }

        let deadline = Instant::now() + wait.limit();
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

fn read_in_background(
    pipe: Option<impl std::io::Read + Send + 'static>,
) -> std::thread::JoinHandle<Zeroizing<Vec<u8>>> {
    std::thread::spawn(move || {
        // room for an item with its secrets, so that the buffer rarely
        // grows and leaves copies behind
        let mut output = Zeroizing::new(Vec::with_capacity(64 * 1024));
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut output);
        }
        output
    })
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
            error_from(r#"{"code":"something_new","message":"m"}"#),
            Error::Other("m".into())
        );
        assert_eq!(
            error_from("error: unexpected argument\n"),
            Error::Other("error: unexpected argument".into())
        );
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
}
