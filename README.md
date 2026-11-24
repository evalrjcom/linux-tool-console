# Linux Tool Console

Linux Tool Console is a native Rust and FLTK launcher for graphical and command-line Linux utilities.

## Run

The launcher requires an X11 or XWayland session.

To build from source, install Rust 1.88 or later, CMake 3.28 or later, a C++ compiler, and the X11 and Xft development libraries. On Debian and Ubuntu, the native dependencies are:

```sh
sudo apt install build-essential cmake pkg-config libx11-dev libxext-dev \
  libxft-dev libxinerama-dev libxcursor-dev libxrender-dev libxfixes-dev \
  libfontconfig-dev
cargo build --locked --release
./target/release/linux-tool-console
```

The prebuilt x86-64 binary, when supplied, requires glibc 2.39 or later and the X11 and Xft runtime libraries. Launch it with `./bin/linux-tool-console` from the extracted bundle.

## Use

- Filter by name or path. The Featured view starts with curated tools available on your system. All tools includes executable files from `/usr/bin`, `/bin`, `/usr/local/bin`, and `/sbin`. Browse can select any executable file.
- Enter arguments with shell-style quoting. For Bash, try `-c "uname -a"`. For Network Interfaces, try `addr`. Select **Run and capture** or press Enter in the Arguments field. Capture streams stdout and stderr, reports the exit code or terminating signal, and gives stdin immediate EOF. The display retains up to 512 KiB of recent output and indicates when it omits earlier bytes. UTF-8 decoding replaces invalid sequences.
- Select **Open in its own window** to launch a desktop app or start a terminal session. **Use terminal** controls this behavior. The launcher selects an initial setting from available desktop entries and featured tools.
- Select **Stop capture** to terminate the captured process group, including commands started by a shell in the same process group. After the leader exits, capture drains inherited pipes briefly and reports if they stayed open.
- Select **Open as root** to start a separate, labeled root session. Authenticate through PolicyKit or sudo. An available terminal emulator can provide a text prompt. Root terminal launches require a supported emulator that starts a local process. Select and run tools again in the root session to give them elevated privileges. Desktop apps may have their own restrictions on running as root.

Available tools vary with the distribution and installed packages. Shell aliases and package-managed apps with no executable file in the scanned directories may require Browse or a direct command through Bash.

## Checks

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```
