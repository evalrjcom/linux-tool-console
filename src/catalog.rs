use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const BIN_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/local/bin", "/sbin"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Desktop,
    Terminal,
}

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub path: PathBuf,
    pub description: &'static str,
    pub args: &'static str,
    pub mode: Mode,
    pub featured: bool,
    pub icon: &'static str,
}

type FeaturedEntry = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    Mode,
    &'static str,
);

const FEATURED: &[FeaturedEntry] = &[
    (
        &["bash"],
        "Bash",
        "Run shell commands or start an interactive Bash session.",
        "",
        Mode::Terminal,
        "shell",
    ),
    (
        &[
            "gnome-text-editor",
            "mousepad",
            "kate",
            "gedit",
            "xed",
            "leafpad",
            "geany",
        ],
        "Text Editor",
        "Open and edit plain-text files in your desktop editor.",
        "",
        Mode::Desktop,
        "file",
    ),
    (
        &[
            "gnome-calculator",
            "kcalc",
            "mate-calc",
            "galculator",
            "xcalc",
        ],
        "Calculator",
        "Open your desktop calculator.",
        "",
        Mode::Desktop,
        "calc",
    ),
    (
        &[
            "gnome-system-monitor",
            "xfce4-taskmanager",
            "plasma-systemmonitor",
            "mate-system-monitor",
        ],
        "System Monitor",
        "Inspect processes and system resource usage.",
        "",
        Mode::Desktop,
        "graph",
    ),
    (
        &["top"],
        "System Monitor",
        "Inspect running processes and resource usage. Capture takes one snapshot.",
        "-b -n 1",
        Mode::Terminal,
        "graph",
    ),
    (
        &["gnome-disks"],
        "Disks",
        "Inspect storage devices and partitions.",
        "",
        Mode::Desktop,
        "disk",
    ),
    (
        &["lsblk"],
        "Disks",
        "List block devices, partitions, and mount points.",
        "",
        Mode::Terminal,
        "disk",
    ),
    (
        &["journalctl"],
        "System Logs",
        "Read recent entries from the systemd journal.",
        "-n 50 --no-pager",
        Mode::Terminal,
        "file",
    ),
    (
        &["zsh"],
        "Zsh",
        "Run shell commands or start an interactive Zsh session.",
        "",
        Mode::Terminal,
        "shell",
    ),
    (
        &["ip"],
        "Network Interfaces",
        "Show interface addresses and network configuration.",
        "addr",
        Mode::Terminal,
        "net",
    ),
    (
        &["thunar", "nautilus", "dolphin", "pcmanfm", "nemo"],
        "File Manager",
        "Browse files and folders on your desktop.",
        "",
        Mode::Desktop,
        "file",
    ),
    (
        &["df"],
        "Disk Usage",
        "Show available space on mounted file systems.",
        "-h",
        Mode::Terminal,
        "disk",
    ),
    (
        &["free"],
        "Memory Usage",
        "Display used and available memory.",
        "-h",
        Mode::Terminal,
        "graph",
    ),
    (
        &["uname"],
        "System Information",
        "Print the kernel version and machine architecture.",
        "-a",
        Mode::Terminal,
        "graph",
    ),
    (
        &["ps"],
        "Processes",
        "List running processes.",
        "-ef",
        Mode::Terminal,
        "shell",
    ),
];

pub fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    std::ffi::CString::new(path.as_os_str().as_bytes()).is_ok_and(|path| unsafe {
        libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::X_OK, libc::AT_EACCESS) == 0
    })
}

pub fn find(name: &str) -> Option<PathBuf> {
    BIN_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| is_executable(path))
}

pub fn scan() -> Vec<Tool> {
    let desktop_modes = desktop_modes();
    let mut catalog = Vec::new();
    let mut names = HashSet::new();

    for &(candidates, name, description, args, mode, icon) in FEATURED {
        if names.contains(name) {
            continue;
        }
        if let Some(path) = candidates.iter().find_map(|candidate| find(candidate)) {
            names.insert(name);
            catalog.push(Tool {
                name: name.into(),
                path,
                description,
                args,
                mode,
                featured: true,
                icon,
            });
        }
    }

    // /bin and /usr/bin are frequently aliases. Keep distinct executable names
    // even when they dispatch through the same multicall binary.
    let mut seen = HashSet::new();
    let mut discovered = Vec::new();
    for dir in BIN_DIRS {
        let Ok(directory) = fs::read_dir(dir) else {
            continue;
        };
        for entry in directory.flatten() {
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }
            let identity = (
                entry.file_name(),
                path.canonicalize().unwrap_or_else(|_| path.clone()),
            );
            if seen.insert(identity) {
                discovered.push(tool_from_path(path, &desktop_modes));
            }
        }
    }
    discovered.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then(a.path.cmp(&b.path))
    });
    catalog.extend(discovered);
    catalog
}

fn desktop_modes() -> HashMap<String, Mode> {
    let mut modes = HashMap::new();
    for dir in ["/usr/share/applications", "/usr/local/share/applications"] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for file in entries
            .flatten()
            .filter(|f| f.path().extension().is_some_and(|ext| ext == "desktop"))
        {
            let Ok(data) = fs::read_to_string(file.path()) else {
                continue;
            };
            let section = data
                .lines()
                .map(str::trim)
                .skip_while(|line| *line != "[Desktop Entry]")
                .skip(1)
                .take_while(|line| !line.starts_with('['))
                .collect::<Vec<_>>();
            if !section.contains(&"Type=Application") {
                continue;
            }
            let mode = if section.contains(&"Terminal=true") {
                Mode::Terminal
            } else {
                Mode::Desktop
            };
            if let Some(exec) = section
                .iter()
                .find_map(|line| line.strip_prefix("Exec="))
                .and_then(|line| shell_words::split(line).ok())
                .and_then(|words| words.into_iter().next())
                .and_then(|path| {
                    Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
            {
                modes
                    .entry(exec)
                    .and_modify(|value| {
                        if mode == Mode::Desktop {
                            *value = mode;
                        }
                    })
                    .or_insert(mode);
            }
        }
    }
    modes
}

fn tool_from_path(path: PathBuf, modes: &HashMap<String, Mode>) -> Tool {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    let mode = modes.get(&name).copied().unwrap_or(Mode::Terminal);
    Tool {
        name,
        path,
        description: "Choose arguments or open this executable in its own window.",
        args: "",
        mode,
        featured: false,
        icon: if mode == Mode::Desktop {
            "window"
        } else {
            "shell"
        },
    }
}

pub fn browsed(path: PathBuf) -> Tool {
    tool_from_path(path, &desktop_modes())
}
