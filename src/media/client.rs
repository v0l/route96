use log::debug;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::protocol::{Job, JobError, Meta, Request, Response, INPUT_FD, read_frame, write_frame};

pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_MEMORY_MB: u64 = 2048;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
const STDERR_CAPTURE_BYTES: usize = 4096;

static TIMEOUT_SECS: AtomicU64 = AtomicU64::new(DEFAULT_TIMEOUT_SECS);
static MEMORY_MB: AtomicU64 = AtomicU64::new(DEFAULT_MEMORY_MB);

pub fn configure(timeout_secs: Option<u64>, memory_mb: Option<u64>) {
    TIMEOUT_SECS.store(
        timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).max(1),
        Ordering::Relaxed,
    );
    MEMORY_MB.store(
        memory_mb.unwrap_or(DEFAULT_MEMORY_MB).max(64),
        Ordering::Relaxed,
    );
}

#[derive(Debug, Clone, PartialEq)]
pub enum RejectReason {
    Timeout,
    Signal(i32),
    ExitCode(i32),
    Malformed(String),
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("timed out"),
            Self::Signal(s) => write!(f, "killed by signal {}", s),
            Self::ExitCode(c) => write!(f, "exited with status {}", c),
            Self::Malformed(m) => write!(f, "malformed response: {}", m),
        }
    }
}

#[derive(Debug, Clone)]
pub enum MediaError {
    Rejected(RejectReason),
    Job(JobError),
    Unavailable(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(r) => write!(f, "media rejected: worker {}", r),
            Self::Job(e) => write!(f, "{}", e),
            Self::Unavailable(m) => write!(f, "media worker unavailable: {}", m),
        }
    }
}

impl std::error::Error for MediaError {}

#[derive(Debug, Clone)]
pub struct MediaSandbox {
    worker: PathBuf,
    timeout: Duration,
    memory_mb: u64,
    namespaces: bool,
    selftest: bool,
}

pub fn worker_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ROUTE96_MEDIA_BIN") {
        return PathBuf::from(p);
    }
    const NAME: &str = "route96-media";
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(2) {
            let candidate = dir.join(NAME);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(NAME)
}

impl Default for MediaSandbox {
    fn default() -> Self {
        Self::from_config()
    }
}

impl MediaSandbox {
    pub fn from_config() -> Self {
        Self {
            worker: worker_path(),
            timeout: Duration::from_secs(TIMEOUT_SECS.load(Ordering::Relaxed)),
            memory_mb: MEMORY_MB.load(Ordering::Relaxed),
            namespaces: true,
            selftest: false,
        }
    }

    pub fn with_worker(mut self, worker: impl Into<PathBuf>) -> Self {
        self.worker = worker.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_memory_mb(mut self, memory_mb: u64) -> Self {
        self.memory_mb = memory_mb;
        self
    }

    pub fn with_namespaces(mut self, namespaces: bool) -> Self {
        self.namespaces = namespaces;
        self
    }

    pub fn with_selftest(mut self, selftest: bool) -> Self {
        self.selftest = selftest;
        self
    }

    pub fn run(&self, input: &Path, job: Job) -> Result<(Meta, Vec<u8>), MediaError> {
        self.run_limited(input, job, DEFAULT_MAX_OUTPUT_BYTES)
    }

    pub fn run_limited(
        &self,
        input: &Path,
        job: Job,
        max_output_bytes: u64,
    ) -> Result<(Meta, Vec<u8>), MediaError> {
        let file = File::open(input)
            .map_err(|e| MediaError::Job(JobError::Failed { message: e.to_string() }))?;
        let source_fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
        if source_fd < 0 {
            return Err(MediaError::Unavailable(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        let source = unsafe { <File as std::os::unix::io::FromRawFd>::from_raw_fd(source_fd) };
        drop(file);

        let mut cmd = Command::new(&self.worker);
        cmd.env_clear()
            .arg("--timeout-secs")
            .arg(self.timeout.as_secs().max(1).to_string())
            .arg("--memory-mb")
            .arg(self.memory_mb.to_string());
        if !self.namespaces {
            cmd.arg("--no-namespaces");
        }
        if self.selftest {
            cmd.arg("--selftest");
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let raw = source.as_raw_fd();
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(raw, INPUT_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| MediaError::Unavailable(format!("{}: {}", self.worker.display(), e)))?;
        drop(source);
        let pid = child.id() as libc::pid_t;

        let request = Request {
            job,
            max_output_bytes,
        };
        let mut stdin = child.stdin.take().expect("piped stdin");
        let write_ok = write_frame(&mut stdin, &request).is_ok();
        drop(stdin);

        let mut stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(read_response(&mut stdout, max_output_bytes));
        });
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr_thread = std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if kept.len() < STDERR_CAPTURE_BYTES {
                    kept.extend_from_slice(&buf[..n.min(STDERR_CAPTURE_BYTES - kept.len())]);
                }
            }
            String::from_utf8_lossy(&kept).into_owned()
        });

        let deadline = Instant::now() + self.timeout;
        let outcome = rx.recv_timeout(self.timeout);
        let kill_group = || unsafe {
            libc::kill(-pid, libc::SIGKILL);
        };

        let result = match outcome {
            Err(_) => {
                kill_group();
                let _ = child.wait();
                Err(MediaError::Rejected(RejectReason::Timeout))
            }
            Ok(parsed) => match wait_until(&mut child, deadline) {
                None => {
                    kill_group();
                    let _ = child.wait();
                    Err(MediaError::Rejected(RejectReason::Timeout))
                }
                Some(status) => {
                    kill_group();
                    classify(status, parsed, write_ok)
                }
            },
        };
        if let Ok(stderr) = stderr_thread.join()
            && !stderr.trim().is_empty()
        {
            debug!("route96-media: {}", stderr.trim());
        }
        result
    }
}

type Parsed = Result<(Response, Vec<u8>), String>;

fn read_response(stdout: &mut impl Read, max_output: u64) -> Parsed {
    let response: Response = read_frame(stdout).map_err(|e| e.to_string())?;
    let payload_len = match &response {
        Response::Ok { payload_len, .. } => *payload_len,
        Response::Err { .. } => 0,
    };
    if payload_len > max_output {
        return Err(format!("payload of {} bytes exceeds limit", payload_len));
    }
    let mut payload = Vec::new();
    stdout
        .take(payload_len)
        .read_to_end(&mut payload)
        .map_err(|e| e.to_string())?;
    if payload.len() as u64 != payload_len {
        return Err("payload truncated".to_string());
    }
    Ok((response, payload))
}

fn wait_until(child: &mut std::process::Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
}

fn classify(
    status: ExitStatus,
    parsed: Parsed,
    write_ok: bool,
) -> Result<(Meta, Vec<u8>), MediaError> {
    if let Some(sig) = status.signal() {
        return Err(MediaError::Rejected(RejectReason::Signal(sig)));
    }
    if !status.success() {
        return Err(MediaError::Rejected(RejectReason::ExitCode(
            status.code().unwrap_or(-1),
        )));
    }
    if !write_ok {
        return Err(MediaError::Rejected(RejectReason::Malformed(
            "request not accepted".to_string(),
        )));
    }
    match parsed {
        Err(m) => Err(MediaError::Rejected(RejectReason::Malformed(m))),
        Ok((Response::Err { error }, _)) => Err(MediaError::Job(error)),
        Ok((Response::Ok { meta, .. }, payload)) => Ok((meta, payload)),
    }
}
