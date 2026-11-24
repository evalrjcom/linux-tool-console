use crate::catalog;
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const QUEUE_LIMIT: usize = 512 * 1024;
const PIPE_IDLE_GRACE: Duration = Duration::from_millis(300);
const MAX_TAIL_TIME: Duration = Duration::from_secs(2);

pub struct Capture(Arc<Mutex<Job>>);

struct Job {
    pid: Option<u32>,
    stopped: bool,
    pending: Batch,
}

#[derive(Default)]
pub struct Batch {
    pub output: VecDeque<Output>,
    pub omitted: usize,
    pub done: Option<Completion>,
    bytes: usize,
}

pub struct Output {
    pub text: String,
    pub error: bool,
}

pub struct Completion {
    pub message: String,
    pub success: bool,
}

impl Capture {
    pub fn take_output(&self) -> Batch {
        std::mem::take(&mut self.0.lock().unwrap().pending)
    }

    pub fn stop(&self) -> io::Result<bool> {
        self.0.lock().unwrap().stop()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // Signal immediately even if the UI or a descendant's pipe is stalled.
        let _ = self.stop();
    }
}

impl Job {
    fn stop(&mut self) -> io::Result<bool> {
        let Some(pid) = self.pid.filter(|_| !self.stopped) else {
            return Ok(false);
        };
        // The child starts a new process group. The unreaped leader reserves
        // its PID until reap() clears it under the same lock used by Stop.
        if unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(false);
            }
            return Err(error);
        }
        self.stopped = true;
        Ok(true)
    }

    fn publish(&mut self, text: String, error: bool) {
        // A modal dialog or slow redraw must never backpressure the child.
        // Keep a bounded tail and let the UI report dropped bytes.
        self.pending.bytes += text.len();
        self.pending.output.push_back(Output { text, error });
        while self.pending.bytes > QUEUE_LIMIT {
            let len = self.pending.output.pop_front().unwrap().text.len();
            self.pending.bytes -= len;
            self.pending.omitted = self.pending.omitted.saturating_add(len);
        }
    }
}

struct CapturedChild {
    child: Child,
    job: Arc<Mutex<Job>>,
}

impl CapturedChild {
    fn reap(&mut self) -> io::Result<(ExitStatus, bool)> {
        let mut job = self.job.lock().unwrap();
        let result = self.child.wait();
        job.pid = None;
        result.map(|status| (status, job.stopped))
    }
}

impl Drop for CapturedChild {
    fn drop(&mut self) {
        // Covers setup and worker-spawn failures as well as an early return.
        let mut job = self.job.lock().unwrap();
        if job.pid.is_some() {
            let _ = job.stop();
            job.pid = None;
            drop(job);
            let _ = self.child.wait();
        }
    }
}

pub fn prepare_capture() -> io::Result<()> {
    // A launcher can inherit SIGCHLD ignored from its parent. Reset it before
    // spawning anything so Linux retains exited leaders until we reap them.
    if unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) } == libc::SIG_ERR {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn start(path: &Path, args: &[String]) -> io::Result<Capture> {
    let child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let job = Arc::new(Mutex::new(Job {
        pid: Some(child.id()),
        stopped: false,
        pending: Batch::default(),
    }));
    let mut captured = CapturedChild {
        child,
        job: job.clone(),
    };
    let stdout = captured
        .child
        .stdout
        .take()
        .expect("stdout was configured as a pipe");
    let stderr = captured
        .child
        .stderr
        .take()
        .expect("stderr was configured as a pipe");
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        nonblocking(fd)?;
    }
    thread::Builder::new()
        .name("tool-capture".into())
        .spawn(move || {
            capture_worker(captured, Pipe::new(stdout, false), Pipe::new(stderr, true));
        })?;
    Ok(Capture(job))
}

fn capture_worker(
    mut captured: CapturedChild,
    mut out: Pipe<impl Read + AsRawFd>,
    mut err: Pipe<impl Read + AsRawFd>,
) {
    let job = captured.job.clone();
    let mut exited_at = None;
    let mut last_read = Instant::now();
    loop {
        // Read both pipes even if the first had data to avoid starving stderr.
        let read_out = out.drain(&job);
        let read_err = err.drain(&job);
        let now = Instant::now();
        if read_out || read_err {
            last_read = now;
        }
        if exited_at.is_none() {
            match has_exited(captured.child.id()) {
                Ok(true) => {
                    exited_at = Some(now);
                    last_read = now;
                }
                Ok(false) => {}
                Err(error) => {
                    drop(captured);
                    job.lock().unwrap().pending.done = Some(Completion {
                        message: format!("Could not monitor process: {error}"),
                        success: false,
                    });
                    return;
                }
            }
        }
        let pipes_open = !out.closed || !err.closed;
        if let Some(exit_time) = exited_at
            && (!pipes_open
                || last_read.elapsed() >= PIPE_IDLE_GRACE
                || exit_time.elapsed() >= MAX_TAIL_TIME)
        {
            out.finish(&job);
            err.finish(&job);
            let done = match captured.reap() {
                Ok((status, stopped)) => {
                    let mut message = match (status.code(), status.signal()) {
                        (Some(code), _) => format!("Exit code {code}"),
                        (_, Some(signal)) => format!("Terminated by signal {signal}"),
                        _ => status.to_string(),
                    };
                    if stopped {
                        message = format!("Stopped · {message}");
                    }
                    if pipes_open {
                        message.push_str(" · output ended while a descendant pipe was still open");
                    }
                    Completion {
                        message,
                        success: status.success() && !stopped,
                    }
                }
                Err(error) => Completion {
                    message: format!("Could not reap process: {error}"),
                    success: false,
                },
            };
            job.lock().unwrap().pending.done = Some(done);
            return;
        }
        // Sleep until a pipe becomes readable, with a bounded wait for a
        // leader that exits while a descendant keeps its pipes open.
        let mut fds = [
            libc::pollfd {
                fd: if out.closed {
                    -1
                } else {
                    out.reader.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if err.closed {
                    -1
                } else {
                    err.reader.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 30) };
    }
}

fn has_exited(pid: u32) -> io::Result<bool> {
    loop {
        // Observe the leader's exit without releasing its PID while its
        // descendants are still being captured and can be stopped.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(unsafe { info.si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Pipe<R> {
    reader: R,
    pending: Vec<u8>,
    error: bool,
    closed: bool,
}

impl<R: Read> Pipe<R> {
    fn new(reader: R, error: bool) -> Self {
        Self {
            reader,
            pending: Vec::new(),
            error,
            closed: false,
        }
    }

    fn drain(&mut self, job: &Mutex<Job>) -> bool {
        if self.closed {
            return false;
        }
        let mut read = false;
        let mut buffer = [0; 8192];
        for _ in 0..16 {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.closed = true;
                    self.finish(job);
                    break;
                }
                Ok(count) => {
                    read = true;
                    self.pending.extend_from_slice(&buffer[..count]);
                    let count = complete_utf8_prefix(&self.pending);
                    if count > 0 {
                        let text = String::from_utf8_lossy(&self.pending[..count]).into_owned();
                        self.pending.drain(..count);
                        job.lock().unwrap().publish(text, self.error);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.closed = true;
                    self.finish(job);
                    job.lock()
                        .unwrap()
                        .publish(format!("\nCould not read process output: {error}\n"), true);
                    break;
                }
            }
        }
        read
    }

    fn finish(&mut self, job: &Mutex<Job>) {
        if !self.pending.is_empty() {
            let text = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            job.lock().unwrap().publish(text, self.error);
        }
    }
}

fn complete_utf8_prefix(mut bytes: &[u8]) -> usize {
    let mut consumed = 0;
    while let Err(error) = std::str::from_utf8(bytes) {
        let start = error.valid_up_to();
        let Some(length) = error.error_len() else {
            return consumed + start;
        };
        consumed += start + length;
        bytes = &bytes[start + length..];
    }
    consumed + bytes.len()
}

pub fn detached(mut command: Command) -> io::Result<()> {
    let child = quiet_spawn(&mut command)?;
    reap_detached(child);
    Ok(())
}

fn reap_detached(mut child: Child) {
    thread::spawn(move || {
        let _ = child.wait();
    });
}

fn quiet_spawn(command: &mut Command) -> io::Result<Child> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

struct Emulator {
    name: &'static str,
    flag: &'static str,
    options: &'static [&'static str],
    local: bool,
}

const EMULATORS: &[Emulator] = &[
    Emulator {
        name: "xfce4-terminal",
        flag: "--execute",
        options: &["--disable-server"],
        local: true,
    },
    Emulator {
        name: "gnome-terminal",
        flag: "--",
        options: &["--wait"],
        local: false,
    },
    Emulator {
        name: "kgx",
        flag: "--",
        options: &[],
        local: false,
    },
    Emulator {
        name: "mate-terminal",
        flag: "-x",
        options: &[],
        local: false,
    },
    Emulator {
        name: "konsole",
        flag: "-e",
        options: &["--separate"],
        local: true,
    },
    Emulator {
        name: "kitty",
        flag: "--",
        options: &[],
        local: true,
    },
    Emulator {
        name: "alacritty",
        flag: "-e",
        options: &[],
        local: true,
    },
    Emulator {
        name: "x-terminal-emulator",
        flag: "-e",
        options: &[],
        local: false,
    },
    Emulator {
        name: "xterm",
        flag: "-e",
        options: &[],
        local: true,
    },
];

pub fn terminal(program: &OsStr, args: &[OsString], root: bool) -> io::Result<()> {
    let child = start_terminal(program, args, root, false)?;
    reap_detached(child);
    Ok(())
}

fn start_terminal(
    program: &OsStr,
    args: &[OsString],
    root: bool,
    authentication: bool,
) -> io::Result<Child> {
    let mut last_error = None;
    for emulator in EMULATORS {
        // A root command must run in a local emulator process. A desktop
        // terminal server can run it under the server's effective UID.
        if root && !emulator.local {
            continue;
        }
        let Some(path) = catalog::find(emulator.name) else {
            continue;
        };
        let mut command = Command::new(path);
        command.args(emulator.options).arg(emulator.flag);
        // Keep transient output visible. Command and argv are positional
        // parameters so the wrapper never interprets their contents.
        command.args(["/bin/sh", "-c",
            r#""$@"; result=$?; printf '\nExit code %s. Press Enter to close… ' "$result"; IFS= read -r reply; exit "$result""#,
            "linux-tool-console"]);
        command.arg(program).args(args);
        if authentication {
            command.current_dir("/");
        }
        match quiet_spawn(&mut command) {
            Ok(child) => return Ok(child),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            if root {
                "No supported local terminal emulator was found for the root session"
            } else {
                "No supported terminal emulator was found"
            },
        )
    }))
}

pub fn elevated() -> io::Result<Child> {
    let args = elevation_args()?;
    let find_auth = |name: &str| {
        ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
            .iter()
            .map(|dir| Path::new(dir).join(name))
            .find(|path| catalog::is_executable(path))
    };
    if let Some(pkexec) = find_auth("pkexec") {
        // A TTY lets pkexec use its own text agent when the desktop has
        // no PolicyKit agent. Never restart authentication after a denial.
        return match start_terminal(pkexec.as_os_str(), &args, false, true) {
            Ok(child) => Ok(child),
            Err(_) => quiet_spawn(
                Command::new(pkexec)
                    .current_dir("/")
                    .arg("--disable-internal-agent")
                    .args(&args),
            ),
        };
    }
    let sudo = find_auth("sudo").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Neither pkexec nor sudo is available",
        )
    })?;
    let mut sudo_args = vec![OsString::from("-H"), OsString::from("--")];
    sudo_args.extend(args);
    start_terminal(sudo.as_os_str(), &sudo_args, false, true)
}

fn elevation_args() -> io::Result<Vec<OsString>> {
    let display = std::env::var_os("DISPLAY")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::other("An X11 display is required for the privileged window"))?;
    let authority = std::env::var_os("XAUTHORITY")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let path = std::path::PathBuf::from(std::env::var_os("HOME")?).join(".Xauthority");
            path.is_file().then_some(path)
        });
    let mut args = vec![OsString::from("/usr/bin/env")];
    let mut assignment = OsString::from("DISPLAY=");
    assignment.push(display);
    args.push(assignment);
    if let Some(path) = authority {
        let mut assignment = OsString::from("XAUTHORITY=");
        assignment.push(std::path::absolute(path)?);
        args.push(assignment);
    }
    args.push(std::env::current_exe()?.into_os_string());
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(script: &str) -> Capture {
        start(Path::new("/bin/bash"), &["-c".into(), script.into()]).unwrap()
    }

    fn until<T>(mut get: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(value) = get() {
                return value;
            }
            assert!(Instant::now() < deadline, "capture timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn streams_both_pipes_handles_invalid_utf8_and_gives_stdin_eof() {
        let capture = bash(
            r#"if read -r line; then exit 99; fi; printf '\342'; sleep .06; printf '\202\254\377'; printf 'error' >&2; exit 7"#,
        );
        let (mut stdout, mut stderr) = (String::new(), String::new());
        let done = until(|| {
            let batch = capture.take_output();
            for output in batch.output {
                if output.error {
                    stderr.push_str(&output.text);
                } else {
                    stdout.push_str(&output.text);
                }
            }
            batch.done
        });
        assert_eq!(done.message, "Exit code 7");
        assert!(!done.success);
        assert_eq!(stdout, "€�");
        assert_eq!(stderr, "error");
    }

    #[test]
    fn stop_signals_group_even_after_leader_exits() {
        let capture = bash("sleep 60 & exit 0");
        let pid = capture.0.lock().unwrap().pid.unwrap();
        until(|| has_exited(pid).unwrap().then_some(()));
        assert!(capture.stop().unwrap());
        let done = until(|| capture.take_output().done);
        assert!(done.message.starts_with("Stopped · Exit code 0"));
        assert!(!done.success);
        assert!(!capture.stop().unwrap());
    }

    #[test]
    fn drains_short_lived_descendant_output_before_completion() {
        let capture = bash("(sleep .12; printf tail) & exit 3");
        let mut stdout = String::new();
        let done = until(|| {
            let batch = capture.take_output();
            for output in batch.output {
                stdout.push_str(&output.text);
            }
            batch.done
        });
        assert_eq!(done.message, "Exit code 3");
        assert_eq!(stdout, "tail");
    }

    #[test]
    fn dropping_capture_signals_a_running_process() {
        let capture = bash("exec sleep 60");
        let job = capture.0.clone();
        drop(capture);
        let done = until(|| job.lock().unwrap().pending.done.take());
        assert_eq!(done.message, "Stopped · Terminated by signal 9");
        assert!(!done.success);
    }

    #[test]
    fn slow_consumer_never_blocks_a_noisy_child() {
        let capture = bash("head -c 1500000 /dev/zero | tr '\\0' x; printf 'done' >&2");
        until(|| {
            capture
                .0
                .lock()
                .unwrap()
                .pending
                .done
                .is_some()
                .then_some(())
        });
        let batch = capture.take_output();
        assert!(batch.omitted > 0);
        assert!(batch.bytes <= QUEUE_LIMIT);
        assert!(
            batch
                .output
                .iter()
                .any(|output| output.text == "done" && output.error)
        );
        assert!(batch.done.unwrap().success);
    }
}
