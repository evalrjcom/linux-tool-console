use crate::catalog;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

pub struct Capture {
    pub stop: Arc<AtomicBool>,
    pub events: Receiver<Event>,
}

pub enum Event {
    Output { text: String, error: bool },
    Done { message: String, success: bool },
}

pub fn start(path: &Path, args: &[String]) -> io::Result<Capture> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        if let Err(error) = nonblocking(fd) {
            kill_group(child.id());
            let _ = child.wait();
            return Err(error);
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let cancelled = stop.clone();
    let (tx, events) = sync_channel(128);
    thread::spawn(move || {
        let mut out = Pipe::new(stdout, false);
        let mut err = Pipe::new(stderr, true);
        let pid = child.id();
        let mut exit = None;
        let mut was_stopped = false;
        loop {
            if exit.is_none() && !was_stopped && cancelled.load(Ordering::Relaxed) {
                kill_group(pid);
                was_stopped = true;
            }
            if !out.drain(&tx) || !err.drain(&tx) {
                if exit.is_none() {
                    kill_group(pid);
                    let _ = child.wait();
                }
                return;
            }
            if exit.is_none() {
                match child.try_wait() {
                    Ok(Some(status)) => exit = Some((status, Instant::now())),
                    Ok(None) => {}
                    Err(error) => {
                        kill_group(pid);
                        let _ = child.wait();
                        let _ = tx.send(Event::Done {
                            message: format!("Could not wait for process: {error}"),
                            success: false,
                        });
                        return;
                    }
                }
            }
            if let Some((status, time)) = exit
                && (out.closed && err.closed || time.elapsed() >= Duration::from_millis(300))
            {
                out.finish(&tx);
                err.finish(&tx);
                let mut message = match (status.code(), status.signal()) {
                    (Some(code), _) => format!("Exit code {code}"),
                    (_, Some(signal)) => format!("Terminated by signal {signal}"),
                    _ => status.to_string(),
                };
                if was_stopped {
                    message = format!("Stopped · {message}");
                }
                if !out.closed || !err.closed {
                    message.push_str(" · descendant pipe still open");
                }
                let _ = tx.send(Event::Done {
                    message,
                    success: status.success() && !was_stopped,
                });
                return;
            }
            thread::sleep(Duration::from_millis(12));
        }
    });
    Ok(Capture { stop, events })
}

fn nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn kill_group(pid: u32) {
    // The captured child starts a new process group. A negative PID targets
    // that group so a shell's foreground commands are also stopped.
    unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
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

    fn drain(&mut self, tx: &SyncSender<Event>) -> bool {
        if self.closed {
            return true;
        }
        let mut buffer = [0; 8192];
        for _ in 0..16 {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.closed = true;
                    return self.finish(tx);
                }
                Ok(count) => {
                    self.pending.extend_from_slice(&buffer[..count]);
                    let count = complete_utf8_prefix(&self.pending);
                    if count > 0 {
                        let text = String::from_utf8_lossy(&self.pending[..count]).into_owned();
                        self.pending.drain(..count);
                        if tx
                            .send(Event::Output {
                                text,
                                error: self.error,
                            })
                            .is_err()
                        {
                            return false;
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.closed = true;
                    return self.finish(tx)
                        && tx
                            .send(Event::Output {
                                text: format!("\nCould not read process output: {error}\n"),
                                error: true,
                            })
                            .is_ok();
                }
            }
        }
        true
    }

    fn finish(&mut self, tx: &SyncSender<Event>) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        tx.send(Event::Output {
            text,
            error: self.error,
        })
        .is_ok()
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
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn terminal(program: &OsStr, args: &[OsString]) -> io::Result<()> {
    for (name, flag, prefix) in [
        ("xfce4-terminal", "--execute", &["--disable-server"][..]),
        ("kgx", "--", &[][..]),
        ("gnome-terminal", "--", &[][..]),
        ("mate-terminal", "-x", &[][..]),
        ("konsole", "-e", &[][..]),
        ("kitty", "--", &[][..]),
        ("alacritty", "-e", &[][..]),
        ("x-terminal-emulator", "-e", &[][..]),
        ("xterm", "-e", &[][..]),
    ] {
        let Some(path) = catalog::find(name) else {
            continue;
        };
        let mut command = Command::new(path);
        command.args(prefix).arg(flag);
        // The wrapper keeps transient CLI output visible. The program and
        // arguments remain positional parameters so their contents are literal.
        command.args(["/bin/sh", "-c",
            r#""$@"; result=$?; printf '\nExit code %s. Press Enter to close… ' "$result"; IFS= read -r reply; exit "$result""#,
            "linux-tool-console"]);
        command.arg(program).args(args);
        return detached(command);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "No supported terminal emulator was found",
    ))
}

pub fn elevated() -> io::Result<Option<Child>> {
    let args = elevation_args()?;
    if let Some(pkexec) = catalog::find("pkexec") {
        return Command::new(pkexec)
            .arg("--disable-internal-agent")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(Some);
    }
    sudo(&args)?;
    Ok(None)
}

pub fn elevated_with_sudo() -> io::Result<()> {
    sudo(&elevation_args()?)
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

fn sudo(args: &[OsString]) -> io::Result<()> {
    let sudo = catalog::find("sudo").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,
        "sudo is unavailable. Install sudo or use pkexec with a PolicyKit authentication agent."))?;
    let mut command = vec![OsString::from("-H"), OsString::from("--")];
    command.extend_from_slice(args);
    terminal(sudo.as_os_str(), &command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_both_pipes_and_reports_nonzero_exit() {
        let args = [
            "-c".into(),
            "printf '\\342'; sleep .06; printf '\\202\\254'; printf 'error' >&2; exit 7".into(),
        ];
        let capture = start(Path::new("/bin/bash"), &args).unwrap();
        let (mut stdout, mut stderr) = (String::new(), String::new());
        loop {
            match capture.events.recv_timeout(Duration::from_secs(3)).unwrap() {
                Event::Output { text, error: false } => stdout.push_str(&text),
                Event::Output { text, error: true } => stderr.push_str(&text),
                Event::Done { message, success } => {
                    assert_eq!(message, "Exit code 7");
                    assert!(!success);
                    break;
                }
            }
        }
        assert_eq!(stdout, "€");
        assert_eq!(stderr, "error");
    }

    #[test]
    fn stops_a_shell_and_its_foreground_command() {
        let args = ["-c".into(), "printf ready; sleep 60".into()];
        let capture = start(Path::new("/bin/bash"), &args).unwrap();
        assert!(matches!(
            capture.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Output { .. }
        ));
        capture.stop.store(true, Ordering::Relaxed);
        assert!(
            matches!(capture.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Done { message, success: false } if message.starts_with("Stopped"))
        );
    }
}
