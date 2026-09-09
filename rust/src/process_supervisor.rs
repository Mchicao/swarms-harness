//! Cross-platform supervision for external worker and verification processes.
//!
//! The runtime owns process lifecycle. Models may decide what work to do, but
//! they must not be relied on to recover a hung wrapper, bound log growth, or
//! reap descendants after cancellation.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum ProcessTerminalReason {
    Exited { code: Option<i32>, success: bool },
    ProviderCompleted,
    TimedOut,
    IdleTimedOut,
    OutputLimitExceeded,
    WaitFailed(String),
}

impl ProcessTerminalReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Exited { .. } => "exited",
            Self::ProviderCompleted => "provider_completed",
            Self::TimedOut => "runtime_timeout",
            Self::IdleTimedOut => "runtime_idle_timeout",
            Self::OutputLimitExceeded => "runtime_output_limit",
            Self::WaitFailed(_) => "runtime_wait_failed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessPolicy {
    pub deadline: Option<Duration>,
    pub idle_deadline: Option<Duration>,
    pub output_limit_bytes: u64,
    pub provider_terminal_grace: Duration,
    pub poll_interval: Duration,
    pub terminate_grace: Duration,
}

impl ProcessPolicy {
    pub fn worker(deadline: Option<Duration>) -> Self {
        Self {
            deadline,
            idle_deadline: env_duration("SWARMS_IDLE_TIMEOUT_SECONDS"),
            output_limit_bytes: env_u64("SWARMS_PROCESS_OUTPUT_LIMIT_BYTES")
                .unwrap_or(64 * 1024 * 1024),
            provider_terminal_grace: Duration::from_secs(3),
            poll_interval: Duration::from_millis(50),
            terminate_grace: Duration::from_secs(1),
        }
    }

    pub fn verification(deadline: Duration) -> Self {
        Self {
            deadline: Some(deadline),
            idle_deadline: None,
            output_limit_bytes: env_u64("SWARMS_VERIFY_OUTPUT_LIMIT_BYTES")
                .unwrap_or(16 * 1024 * 1024),
            provider_terminal_grace: Duration::ZERO,
            poll_interval: Duration::from_millis(50),
            terminate_grace: Duration::from_secs(1),
        }
    }
}

#[derive(Debug)]
pub struct SupervisedOutcome {
    pub reason: ProcessTerminalReason,
    pub status: Option<ExitStatus>,
    pub elapsed: Duration,
}

fn env_duration(name: &str) -> Option<Duration> {
    env_u64(name)
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse::<u64>().ok()
}

/// Configure a child so a later cancellation can terminate its descendants.
///
/// Unix workers enter their own process group. Windows uses `taskkill /T` at
/// termination time, which follows the process tree without requiring a new
/// runtime dependency. Other platforms fall back to killing the direct child.
pub fn prepare_command(command: &mut Command) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // SAFETY: pre_exec runs in the child immediately before exec. setpgid
        // is async-signal-safe and receives only constant scalar arguments.
        unsafe {
            command.pre_exec(|| {
                extern "C" {
                    fn setpgid(pid: i32, pgid: i32) -> i32;
                }
                if unsafe { setpgid(0, 0) } == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }

    Ok(())
}

pub fn wait_supervised(
    child: &mut Child,
    log_path: &Path,
    policy: ProcessPolicy,
    completion_probe: Option<&dyn Fn() -> bool>,
) -> SupervisedOutcome {
    let started = Instant::now();
    let mut last_progress = started;
    let mut last_log_len = fs::metadata(log_path).map(|meta| meta.len()).unwrap_or(0);
    let mut provider_terminal_seen = None;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return SupervisedOutcome {
                    reason: ProcessTerminalReason::Exited {
                        code: status.code(),
                        success: status.success(),
                    },
                    status: Some(status),
                    elapsed: started.elapsed(),
                };
            }
            Ok(None) => {}
            Err(error) => {
                let _ = terminate_tree(child, policy.terminate_grace);
                return SupervisedOutcome {
                    reason: ProcessTerminalReason::WaitFailed(error.to_string()),
                    status: None,
                    elapsed: started.elapsed(),
                };
            }
        }

        let current_len = fs::metadata(log_path).map(|meta| meta.len()).unwrap_or(0);
        if current_len != last_log_len {
            last_log_len = current_len;
            last_progress = Instant::now();
        }

        if policy.output_limit_bytes > 0 && current_len > policy.output_limit_bytes {
            let _ = terminate_tree(child, policy.terminate_grace);
            return SupervisedOutcome {
                reason: ProcessTerminalReason::OutputLimitExceeded,
                status: None,
                elapsed: started.elapsed(),
            };
        }

        if policy
            .deadline
            .is_some_and(|deadline| started.elapsed() >= deadline)
        {
            let _ = terminate_tree(child, policy.terminate_grace);
            return SupervisedOutcome {
                reason: ProcessTerminalReason::TimedOut,
                status: None,
                elapsed: started.elapsed(),
            };
        }

        if policy
            .idle_deadline
            .is_some_and(|deadline| last_progress.elapsed() >= deadline)
        {
            let _ = terminate_tree(child, policy.terminate_grace);
            return SupervisedOutcome {
                reason: ProcessTerminalReason::IdleTimedOut,
                status: None,
                elapsed: started.elapsed(),
            };
        }

        if completion_probe.is_some_and(|probe| probe()) {
            let seen = provider_terminal_seen.get_or_insert_with(Instant::now);
            if seen.elapsed() >= policy.provider_terminal_grace {
                let _ = terminate_tree(child, policy.terminate_grace);
                return SupervisedOutcome {
                    reason: ProcessTerminalReason::ProviderCompleted,
                    status: None,
                    elapsed: started.elapsed(),
                };
            }
        } else {
            provider_terminal_seen = None;
        }

        thread::sleep(policy.poll_interval);
    }
}

pub fn terminate_tree(child: &mut Child, grace: Duration) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Ok(());
    }

    #[cfg(unix)]
    {
        extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        const SIGTERM: i32 = 15;
        const SIGKILL: i32 = 9;
        let group = -(child.id() as i32);
        // SAFETY: the child was placed in a process group whose pgid equals its
        // pid in `prepare_command`; negative pid targets the whole group.
        unsafe {
            let _ = kill(group, SIGTERM);
        }
        let started = Instant::now();
        while started.elapsed() < grace {
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        unsafe {
            let _ = kill(group, SIGKILL);
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    let _ = child.kill();
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("reap process tree: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::process::Stdio;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_log(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "swarms-supervisor-{label}-{}-{}.log",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    fn long_running_command() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/D", "/S", "/C", "ping -n 30 127.0.0.1 >NUL"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30"]);
            command
        }
    }

    #[test]
    fn deadline_terminates_long_running_process() {
        let log_path = temp_log("deadline");
        let log = File::create(&log_path).unwrap();
        let err = log.try_clone().unwrap();
        let mut command = long_running_command();
        command.stdout(Stdio::from(log)).stderr(Stdio::from(err));
        prepare_command(&mut command).unwrap();
        let mut child = command.spawn().unwrap();

        let mut policy = ProcessPolicy::worker(Some(Duration::from_millis(100)));
        policy.poll_interval = Duration::from_millis(10);
        policy.terminate_grace = Duration::from_millis(50);
        let outcome = wait_supervised(&mut child, &log_path, policy, None);

        assert!(matches!(outcome.reason, ProcessTerminalReason::TimedOut));
        assert!(child.try_wait().unwrap().is_some());
        let _ = fs::remove_file(log_path);
    }

    #[test]
    fn output_limit_terminates_process_before_unbounded_log_growth() {
        let log_path = temp_log("output");
        fs::write(&log_path, vec![b'x'; 1024]).unwrap();
        let log = File::options().append(true).open(&log_path).unwrap();
        let err = log.try_clone().unwrap();
        let mut command = long_running_command();
        command.stdout(Stdio::from(log)).stderr(Stdio::from(err));
        prepare_command(&mut command).unwrap();
        let mut child = command.spawn().unwrap();

        let mut policy = ProcessPolicy::worker(None);
        policy.output_limit_bytes = 32;
        policy.poll_interval = Duration::from_millis(10);
        policy.terminate_grace = Duration::from_millis(50);
        let outcome = wait_supervised(&mut child, &log_path, policy, None);

        assert!(matches!(
            outcome.reason,
            ProcessTerminalReason::OutputLimitExceeded
        ));
        assert!(child.try_wait().unwrap().is_some());
        let _ = fs::remove_file(log_path);
    }
}
