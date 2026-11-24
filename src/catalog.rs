use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::{Path, PathBuf};

const BIN_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/local/bin", "/sbin"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Desktop,
    Terminal,
}

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub path: PathBuf,
    pub display_path: String,
    pub description: &'static str,
    pub args: &'static str,
    pub mode: Mode,
    pub featured: bool,
    pub icon: &'static str,
    path_queries: Vec<String>,
    name_query: String,
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

pub struct Catalog {
    pub tools: Vec<Tool>,
    modes: HashMap<PathBuf, Mode>,
}

impl Catalog {
    pub fn scan(root: bool) -> Self {
        let dirs = BIN_DIRS.map(Path::new);
        let modes = desktop_modes(&application_dirs(root), &dirs);
        let mut discovered = discover(&dirs, &modes);
        discovered.sort_by(|a, b| a.name_query.cmp(&b.name_query).then(a.path.cmp(&b.path)));

        let mut tools = Vec::new();
        let mut names = HashSet::new();
        for &(candidates, name, description, args, mode, icon) in FEATURED {
            if names.contains(name) {
                continue;
            }
            if let Some(index) = candidates.iter().find_map(|candidate| {
                let path = find(candidate)?;
                discovered.iter().position(|tool| tool.path == path)
            }) {
                // Promote the discovered record so its path aliases are kept
                // and All tools contains each executable entry only once.
                let mut tool = discovered.remove(index);
                names.insert(name);
                tool.name = name.into();
                tool.name_query = name.to_lowercase();
                tool.description = description;
                tool.args = args;
                tool.mode = mode;
                tool.featured = true;
                tool.icon = icon;
                tools.push(tool);
            }
        }
        tools.extend(discovered);
        Self { tools, modes }
    }

    pub fn browse(&mut self, path: PathBuf) -> usize {
        if let Some(index) = self.tools.iter().position(|tool| tool.path == path) {
            return index;
        }
        // Keep the exact path the user picked, including intentional symlinks.
        let index = self.tools.len();
        self.tools.push(Tool::from_path(path, &self.modes));
        index
    }
}

impl Tool {
    fn from_path(path: PathBuf, modes: &HashMap<PathBuf, Mode>) -> Self {
        let name = display_os(path.file_name().unwrap_or(path.as_os_str()));
        let display_path = display_os(path.as_os_str());
        let mode = modes
            .get(&entry_path(&path))
            .copied()
            .unwrap_or(Mode::Terminal);
        let name_query = name.to_lowercase();
        let mut tool = Self {
            name,
            path,
            path_queries: vec![display_path.to_lowercase()],
            display_path,
            mode,
            name_query,
            description: "Choose arguments or open this executable in its own window.",
            args: "",
            featured: false,
            icon: if mode == Mode::Desktop {
                "window"
            } else {
                "shell"
            },
        };
        if let Ok(target) = tool.path.canonicalize() {
            tool.add_path_query(&target);
        }
        tool
    }

    fn add_path_query(&mut self, path: &Path) {
        let query = display_os(path.as_os_str()).to_lowercase();
        if !self.path_queries.contains(&query) {
            self.path_queries.push(query);
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        // All strings are cached at discovery, so a filter requires no I/O.
        self.name_query.contains(query) || self.path_queries.iter().any(|path| path.contains(query))
    }
}

fn display_os(value: &OsStr) -> String {
    // Render invalid bytes and control characters explicitly so two paths
    // can be told apart while the launcher retains their original bytes.
    let mut label = String::new();
    for chunk in value.as_bytes().utf8_chunks() {
        for ch in chunk.valid().chars() {
            if ch.is_control() || ch == '\\' {
                label.extend(ch.escape_default());
            } else {
                label.push(ch);
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(label, "\\x{byte:02X}");
        }
    }
    label
}

fn discover(dirs: &[&Path], modes: &HashMap<PathBuf, Mode>) -> Vec<Tool> {
    let mut index = HashMap::<PathBuf, usize>::new();
    let mut tools: Vec<Tool> = Vec::new();
    for &dir in dirs {
        let Ok(directory) = fs::read_dir(dir) else {
            continue;
        };
        let physical_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
        for entry in directory.flatten() {
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }
            // Collapse directory aliases such as /bin, but keep separate
            // symlinks in other directories because argv[0] can matter.
            let key = physical_dir.join(entry.file_name());
            if let Some(&existing) = index.get(&key) {
                tools[existing].add_path_query(&path);
            } else {
                index.insert(key, tools.len());
                tools.push(Tool::from_path(path, modes));
            }
        }
    }
    tools
}

fn entry_path(path: &Path) -> PathBuf {
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(parent) = parent.canonicalize()
    {
        return parent.join(name);
    }
    path.to_owned()
}

fn application_dirs(root: bool) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !root {
        if let Some(home) = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| Path::new(&home).join(".local/share"))
                    .filter(|path| path.is_absolute())
            })
        {
            dirs.push(home);
        }
        if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS").filter(|value| !value.is_empty())
        {
            dirs.extend(std::env::split_paths(&data_dirs).filter(|path| path.is_absolute()));
            return dirs
                .into_iter()
                .map(|dir| dir.join("applications"))
                .collect();
        }
    }
    dirs.extend([
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]);
    dirs.into_iter()
        .map(|dir| dir.join("applications"))
        .collect()
}

fn desktop_modes(dirs: &[PathBuf], bins: &[&Path]) -> HashMap<PathBuf, Mode> {
    let mut modes = HashMap::new();
    let mut ids = HashSet::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for file in entries
            .flatten()
            .filter(|f| f.path().extension().is_some_and(|ext| ext == "desktop"))
        {
            if !ids.insert(file.file_name()) {
                continue;
            }
            // Desktop entries supply hints only. Never execute their contents
            // and avoid waiting on special files in user data directories.
            let Ok(data) = read_desktop_entry(&file.path()) else {
                continue;
            };
            if let Some((path, mode)) = desktop_entry(&data, bins) {
                modes
                    .entry(path)
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

fn read_desktop_entry(path: &Path) -> std::io::Result<String> {
    const MAX_SIZE: u64 = 128 * 1024;
    // Check the opened file and limit reads: a desktop hint may be a
    // renamed FIFO, device, or oversized file in a user data directory.
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_SIZE {
        return Err(std::io::Error::other("Invalid desktop entry file"));
    }
    let mut data = String::new();
    if file.take(MAX_SIZE + 1).read_to_string(&mut data)? as u64 > MAX_SIZE {
        return Err(std::io::Error::other("Desktop entry exceeds size limit"));
    }
    Ok(data)
}

fn desktop_entry(data: &str, bins: &[&Path]) -> Option<(PathBuf, Mode)> {
    let mut fields = HashMap::new();
    for line in data
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "[Desktop Entry]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
    {
        if !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            fields.insert(key.trim(), value.trim());
        }
    }
    if fields.get("Type") != Some(&"Application") || fields.get("Hidden") == Some(&"true") {
        return None;
    }
    let args = shell_words::split(fields.get("Exec")?).ok()?;
    let mut tokens = args.iter();
    let mut command = tokens.next()?.as_str();
    if matches!(command, "env" | "/usr/bin/env" | "/bin/env") {
        // Recognize simple environment wrappers without classifying env
        // itself or interpreting shell snippets as executable names.
        command = tokens.find(|arg| !arg.contains('='))?.as_str();
        if command == "--" {
            command = tokens.next()?.as_str();
        }
        if command.starts_with('-') {
            return None;
        }
    }
    let resolve = |name: &str| {
        let path = Path::new(name);
        if path.is_absolute() {
            is_executable(path).then(|| path.to_owned())
        } else if path.file_name() == Some(OsStr::new(name)) {
            bins.iter()
                .map(|dir| dir.join(name))
                .find(|path| is_executable(path))
        } else {
            None
        }
    };
    if let Some(try_exec) = fields.get("TryExec") {
        resolve(try_exec)?;
    }
    let path = resolve(command)?;
    let mode = if fields.get("Terminal") == Some(&"true") {
        Mode::Terminal
    } else {
        Mode::Desktop
    };
    Some((entry_path(&path), mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let tick = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir =
                std::env::temp_dir().join(format!("tool-console-{}-{tick}", std::process::id()));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn executable(&self, path: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn directory_aliases_are_searchable_without_merging_distinct_links() {
        let fixture = Fixture::new();
        let tool = fixture.executable("primary/demo");
        let alias = fixture.0.join("alias");
        symlink(tool.parent().unwrap(), &alias).unwrap();
        let alternate = fixture.0.join("alternate");
        fs::create_dir(&alternate).unwrap();
        symlink(&tool, alternate.join("demo")).unwrap();
        fs::write(alternate.join("non-executable"), "").unwrap();
        let tools = discover(
            &[tool.parent().unwrap(), &alias, &alternate],
            &HashMap::new(),
        );
        assert_eq!(tools.len(), 2);
        // Filtering uses the snapshot even if an installation changes.
        fs::remove_file(&alias).unwrap();
        assert!(tools[0].matches("/alias/demo"));
        assert_eq!(tools[0].path, tool);
        assert_eq!(tools[1].path, alternate.join("demo"));
    }

    #[test]
    fn renders_invalid_filenames_without_conflating_them_with_valid_text() {
        assert_eq!(
            display_os(OsStr::from_bytes(b"picker-\xff")),
            r"picker-\xFF"
        );
        assert_eq!(display_os(OsStr::new("picker-�")), "picker-�");
        assert_eq!(display_os(OsStr::new(r"picker-\xFF")), r"picker-\\xFF");
    }

    #[test]
    fn desktop_modes_use_entry_paths_and_skip_hidden_overrides() {
        let fixture = Fixture::new();
        let tool = fixture.executable("bin/my-tool");
        let other = fixture.executable("other/my-tool");
        let user = fixture.0.join("user");
        let system = fixture.0.join("system");
        fs::create_dir(&user).unwrap();
        fs::create_dir(&system).unwrap();
        let data = format!(
            "[Desktop Entry]\nType=Application\nExec=env KEY=value \"{}\" %U\n",
            tool.display()
        );
        fs::write(system.join("demo.desktop"), &data).unwrap();
        let (path, mode) = desktop_entry(&data, &[tool.parent().unwrap()]).unwrap();
        assert!(mode == Mode::Desktop);
        assert_eq!(path, tool);
        assert_ne!(path, other);
        fs::write(user.join("demo.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(desktop_modes(&[user, system], &[tool.parent().unwrap()]).is_empty());
    }
}
