mod catalog;
mod paint;
mod process;

use catalog::{Mode, Tool};
use fltk::{
    app,
    button::{Button, CheckButton},
    dialog::{self, FileDialogType, NativeFileChooser},
    draw,
    enums::{Align, Color, Event, Font, FrameType, Key},
    frame::Frame,
    input::Input,
    prelude::*,
    text::{StyleTableEntry, TextBuffer, TextDisplay, WrapMode},
    valuator::{Scrollbar, ScrollbarType},
    window::Window,
};
use paint::*;
use std::{
    cell::RefCell,
    ffi::OsString,
    path::PathBuf,
    process::{Child, Command},
    rc::Rc,
    sync::atomic::Ordering,
};

const ROW_HEIGHT: i32 = 49;
const OUTPUT_LIMIT: i32 = 512 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    Featured,
    All,
}

#[derive(Clone, Copy)]
enum Action {
    Search,
    Featured,
    All,
    Scroll,
    Wheel(i32),
    Select(usize),
    Browse,
    Run,
    Stop,
    Open,
    Root,
    Clear,
}

struct State {
    tools: Vec<Tool>,
    visible: Vec<usize>,
    selected: Option<usize>,
    filter: Filter,
    scroll: i32,
    capture: Option<process::Capture>,
    elevation: Option<Child>,
}

#[derive(Clone)]
struct Ui {
    header: Frame,
    logo: Frame,
    title: Frame,
    privilege: Frame,
    root: Button,
    side: Frame,
    catalog: Frame,
    count: Frame,
    search_back: Frame,
    search: Input,
    tabs_back: Frame,
    featured: Button,
    all: Button,
    list: Frame,
    scrollbar: Scrollbar,
    browse: Button,
    details: Frame,
    tool_icon: Frame,
    kind: Frame,
    name: Frame,
    path: Frame,
    description: Frame,
    arguments_label: Frame,
    arguments_back: Frame,
    args: Input,
    run: Button,
    open: Button,
    stop: Button,
    terminal: CheckButton,
    output_panel: Frame,
    output_header: Frame,
    output_title: Frame,
    output_state: Frame,
    clear: Button,
    output: TextDisplay,
    empty: Frame,
    footer: Frame,
    footer_status: Frame,
    footer_note: Frame,
    buffer: TextBuffer,
    styles: TextBuffer,
}

fn main() {
    let application = app::App::default().with_scheme(app::Scheme::Gtk);
    app::background(246, 244, 239);
    app::foreground(37, 41, 36);
    app::set_color(Color::Selection, 169, 64, 38);

    let root = unsafe { libc::geteuid() } == 0;
    let tools = catalog::scan();
    let state = Rc::new(RefCell::new(State {
        tools,
        visible: Vec::new(),
        selected: None,
        filter: Filter::Featured,
        scroll: 0,
        capture: None,
        elevation: None,
    }));

    let (sender, receiver) = app::channel::<Action>();
    let mut window = Window::new(
        0,
        0,
        1120,
        752,
        if root {
            "Linux Tool Console — root"
        } else {
            "Linux Tool Console"
        },
    )
    .center_screen();
    window.set_color(PAPER);
    window.size_range(840, 630, 0, 0);
    let mut ui = Ui::new(state.clone(), root, sender);
    window.end();
    window.resizable(&window.clone());
    let mut layout = ui.clone();
    let resize_state = state.clone();
    window.resize_callback(move |_, _, _, w, h| {
        layout.layout(w, h);
        if let Ok(mut state) = resize_state.try_borrow_mut() {
            layout.update_scroll(&mut state);
        }
    });
    ui.layout(window.w(), window.h());
    ui.refresh_catalog(&mut state.borrow_mut());
    ui.refresh_selection(&state.borrow());
    ui.sync_controls(&state.borrow());
    ui.search.take_focus().ok();

    let close_state = state.clone();
    window.set_callback(move |window| {
        // Dropping the receiver makes the worker cancel and reap its child.
        if let Some(job) = &close_state.borrow().capture {
            job.stop.store(true, Ordering::Relaxed);
        }
        window.hide();
    });

    // Wake FLTK regularly while child events and elevation status are available.
    app::add_timeout3(0.035, |timer| app::repeat_timeout3(0.035, timer));
    window.show();
    while application.wait() {
        while let Some(action) = receiver.recv() {
            if matches!(action, Action::Browse) {
                if let Some(path) = choose_executable() {
                    ui.browse(&mut state.borrow_mut(), path);
                }
            } else {
                ui.act(action, &mut state.borrow_mut());
            }
        }
        ui.poll(&mut state.borrow_mut());
    }
    if let Some(capture) = state.borrow_mut().capture.take() {
        capture.stop.store(true, Ordering::Relaxed);
        while let Ok(event) = capture.events.recv() {
            if matches!(event, process::Event::Done { .. }) {
                break;
            }
        }
    }
}

impl Ui {
    fn new(state: Rc<RefCell<State>>, root: bool, sender: app::Sender<Action>) -> Self {
        let mut header = Frame::default();
        header.draw(|f| {
            draw::draw_rect_fill(f.x(), f.y(), f.w(), f.h(), CARD);
            line(0, f.h() - 1, f.w(), f.h() - 1, BORDER);
        });
        let mut logo = Frame::default();
        logo.draw(|f| {
            rounded(f.x(), f.y(), f.w(), f.h(), 9, DARK, DARK);
            icon("shell", f.x() + 12, f.y() + 11, LIGHT);
        });
        let title = label("Linux Tool Console", Font::HelveticaBold, 23, INK);
        let privilege = label(
            if root { "Root session" } else { "User session" },
            Font::Helvetica,
            13,
            if root { RUST } else { MUTED },
        );
        let mut root_button = button("Open as root", "lock", ButtonStyle::Neutral);
        if root {
            root_button.hide();
        }
        bind(&mut root_button, sender, Action::Root);

        let side = panel(CARD, BORDER);
        let catalog = label("Tool catalog", Font::HelveticaBold, 21, INK);
        let count = label("", Font::Helvetica, 12, MUTED);
        let mut search_back = field_back();
        search_back.draw(|f| {
            rounded(f.x(), f.y(), f.w(), f.h(), 7, CARD, BORDER);
            icon("search", f.x() + 11, f.y() + 11, MUTED);
        });
        let mut search = input();
        search.super_draw(true);
        search.super_draw_first(true);
        search.draw(|field| {
            if field.value().is_empty() {
                draw::set_draw_color(MUTED);
                draw::set_font(Font::Helvetica, 13);
                draw::draw_text2(
                    "Filter by name or path",
                    field.x() + 8,
                    field.y(),
                    field.w() - 10,
                    field.h(),
                    Align::Inside | Align::Left | Align::Clip,
                );
            }
        });
        search.set_trigger(fltk::enums::CallbackTrigger::Changed);
        search.set_tooltip("Filter tools by name or executable path");
        bind(&mut search, sender, Action::Search);
        let mut tabs_back = Frame::default();
        tabs_back.draw(|f| rounded(f.x(), f.y(), f.w(), f.h(), 8, PAPER, PAPER));
        let mut featured = button("Featured", "", ButtonStyle::Tab);
        let mut all = button("All tools", "", ButtonStyle::Tab);
        bind(&mut featured, sender, Action::Featured);
        bind(&mut all, sender, Action::All);
        let list = catalog_list(state, sender);
        let mut scrollbar = Scrollbar::default();
        scrollbar.set_type(ScrollbarType::Vertical);
        scrollbar.set_color(PAPER);
        scrollbar.set_selection_color(BORDER);
        scrollbar.set_linesize(ROW_HEIGHT);
        bind(&mut scrollbar, sender, Action::Scroll);
        let mut browse = button("Browse…", "folder", ButtonStyle::Neutral);
        browse.set_tooltip("Select an executable from any directory");
        bind(&mut browse, sender, Action::Browse);

        let details = panel(CARD, BORDER);
        let mut tool_icon = Frame::default();
        tool_icon.draw(|f| {
            rounded(f.x(), f.y(), f.w(), f.h(), 10, TINT, TINT);
            icon(&f.label(), f.x() + 16, f.y() + 15, RUST);
        });
        let kind = label("", Font::HelveticaBold, 11, RUST);
        let name = label("", Font::HelveticaBold, 28, INK);
        let path = label("", Font::Courier, 13, MUTED);
        let description = label("", Font::Helvetica, 14, MUTED);
        let arguments_label = label("Arguments", Font::HelveticaBold, 13, INK);
        let arguments_back = field_back();
        let mut args = input();
        args.set_text_font(Font::Courier);
        args.set_trigger(fltk::enums::CallbackTrigger::EnterKey);
        bind(&mut args, sender, Action::Run);
        args.set_tooltip(
            "Shell-style quoting is supported. Arguments are passed directly to the executable.",
        );
        let mut run = button("Run and capture", "play", ButtonStyle::Primary);
        let mut open = button("Open in its own window", "open", ButtonStyle::Neutral);
        let mut stop = button("Stop capture", "stop", ButtonStyle::Neutral);
        bind(&mut run, sender, Action::Run);
        bind(&mut open, sender, Action::Open);
        bind(&mut stop, sender, Action::Stop);
        let mut terminal = CheckButton::default().with_label("Use terminal");
        terminal.set_label_color(MUTED);
        terminal.set_label_size(12);
        terminal.set_selection_color(RUST);
        terminal.set_down_frame(FrameType::DownBox);
        terminal.set_tooltip("Open the executable in a terminal emulator for interactive input");

        let output_panel = panel(DARK, DARK);
        let mut output_header = Frame::default();
        output_header.draw(|f| {
            rounded(f.x(), f.y(), f.w(), f.h(), 11, DARK_TOP, DARK_TOP);
            draw::draw_rect_fill(f.x(), f.y() + 20, f.w() + 1, f.h() - 20, DARK_TOP);
            line(
                f.x(),
                f.y() + f.h() - 1,
                f.x() + f.w(),
                f.y() + f.h() - 1,
                DARK_EDGE,
            );
            icon("shell", f.x() + 17, f.y() + 17, DIM_LIGHT);
        });
        let output_title = label("Output", Font::HelveticaBold, 16, LIGHT);
        let mut output_state = label("Idle", Font::Helvetica, 12, DIM_LIGHT);
        output_state.set_align(Align::Right | Align::Inside | Align::Clip);
        let mut clear = button("Clear", "", ButtonStyle::Dark);
        bind(&mut clear, sender, Action::Clear);
        let buffer = TextBuffer::default();
        let styles = TextBuffer::default();
        let mut output = TextDisplay::default();
        output.set_color(DARK);
        output.set_frame(FrameType::FlatBox);
        output.set_text_color(LIGHT);
        output.set_text_font(Font::Courier);
        output.set_text_size(13);
        output.wrap_mode(WrapMode::AtBounds, 0);
        output.set_scrollbar_size(12);
        output.set_buffer(buffer.clone());
        output.set_highlight_data(
            styles.clone(),
            [
                StyleTableEntry {
                    color: LIGHT,
                    font: Font::Courier,
                    size: 13,
                },
                StyleTableEntry {
                    color: Color::from_hex(0xff9f89),
                    font: Font::Courier,
                    size: 13,
                },
                StyleTableEntry {
                    color: Color::from_hex(0xaccca8),
                    font: Font::Courier,
                    size: 13,
                },
            ],
        );
        let mut empty = Frame::default();
        empty.draw(|f| {
            let x = f.x() + f.w() / 2;
            let y = f.y() + f.h() / 2;
            rounded(x - 21, y - 50, 42, 38, 8, DARK_TOP, DARK_EDGE);
            icon("shell", x - 8, y - 40, DIM_LIGHT);
            draw::set_font(Font::HelveticaBold, 16);
            draw::set_draw_color(LIGHT);
            draw::draw_text2("No output yet", f.x(), y, f.w(), 20, Align::Center);
            draw::set_font(Font::Helvetica, 13);
            draw::set_draw_color(DIM_LIGHT);
            draw::draw_text2(
                "Run a tool to see stdout, stderr, and its exit code.",
                f.x(),
                y + 25,
                f.w(),
                20,
                Align::Center,
            );
        });

        let mut footer = Frame::default();
        footer.draw(move |f| {
            line(f.x(), f.y(), f.x() + f.w(), f.y(), BORDER);
            draw::draw_circle_fill(f.x() + 1, f.y() + 12, 7, if root { RUST } else { GREEN });
        });
        let footer_status = label("Ready", Font::Helvetica, 12, MUTED);
        let mut footer_note = label(
            "Capture uses UTF-8 and closes stdin",
            Font::Helvetica,
            12,
            MUTED,
        );
        footer_note.set_align(Align::Right | Align::Inside | Align::Clip);

        Self {
            header,
            logo,
            title,
            privilege,
            root: root_button,
            side,
            catalog,
            count,
            search_back,
            search,
            tabs_back,
            featured,
            all,
            list,
            scrollbar,
            browse,
            details,
            tool_icon,
            kind,
            name,
            path,
            description,
            arguments_label,
            arguments_back,
            args,
            run,
            open,
            stop,
            terminal,
            output_panel,
            output_header,
            output_title,
            output_state,
            clear,
            output,
            empty,
            footer,
            footer_status,
            footer_note,
            buffer,
            styles,
        }
    }

    fn layout(&mut self, w: i32, h: i32) {
        let margin = 24;
        let side_w = 286;
        let x = margin + side_w + 18;
        let main_w = w - x - margin;
        let right = x + main_w;
        let top = 92;
        let bottom = h - 43;
        let output_y = 390;
        let footer_y = h - 33;
        self.header.resize(0, 0, w, 69);
        self.logo.resize(margin, 16, 39, 37);
        self.title.resize(margin + 52, 14, 240, 41);
        self.root.resize(w - margin - 135, 17, 135, 35);
        self.privilege.resize(
            w - margin - if self.root.visible() { 229 } else { 79 },
            18,
            90,
            34,
        );
        self.side.resize(margin, top, side_w, bottom - top);
        self.catalog.resize(margin + 17, top + 15, side_w - 34, 27);
        self.count.resize(margin + 17, top + 44, side_w - 34, 19);
        self.search_back
            .resize(margin + 16, top + 76, side_w - 34, 37);
        self.search.resize(margin + 48, top + 78, side_w - 72, 33);
        self.tabs_back
            .resize(margin + 16, top + 124, side_w - 34, 34);
        self.featured
            .resize(margin + 19, top + 126, (side_w - 40) / 2, 30);
        self.all.resize(
            margin + 19 + (side_w - 40) / 2,
            top + 126,
            (side_w - 40) / 2,
            30,
        );
        let list_y = top + 171;
        self.list
            .resize(margin + 12, list_y, side_w - 39, bottom - list_y - 62);
        self.scrollbar
            .resize(margin + side_w - 24, list_y, 9, bottom - list_y - 62);
        self.browse
            .resize(margin + 16, bottom - 48, side_w - 33, 34);

        self.details.resize(x, top, main_w, output_y - top - 16);
        self.tool_icon.resize(x + 22, top + 24, 47, 46);
        self.kind.resize(x + 82, top + 20, main_w - 104, 15);
        self.name.resize(x + 82, top + 36, main_w - 104, 37);
        self.path.resize(x + 24, top + 84, main_w - 48, 19);
        self.description.resize(x + 24, top + 110, main_w - 48, 22);
        self.arguments_label
            .resize(x + 24, top + 151, main_w - 48, 18);
        self.arguments_back
            .resize(x + 23, top + 176, main_w - 47, 39);
        self.args.resize(x + 31, top + 178, main_w - 63, 35);
        self.run.resize(x + 23, top + 231, 139, 36);
        self.open.resize(x + 170, top + 231, 185, 36);
        self.stop.resize(x + 363, top + 231, 114, 36);
        if main_w >= 610 {
            self.terminal.resize(right - 118, top + 235, 103, 28);
        } else {
            self.terminal.resize(right - 117, top + 145, 102, 27);
        }

        self.output_panel
            .resize(x, output_y, main_w, bottom - output_y);
        self.output_header.resize(x, output_y, main_w - 1, 49);
        self.output_title.resize(x + 41, output_y + 1, 90, 46);
        self.output_state
            .resize(x + 138, output_y + 1, main_w - 229, 46);
        self.clear.resize(right - 77, output_y + 9, 62, 31);
        self.output
            .resize(x + 15, output_y + 58, main_w - 30, bottom - output_y - 73);
        self.empty
            .resize(x + 15, output_y + 50, main_w - 30, bottom - output_y - 59);

        self.footer.resize(margin, footer_y, w - 2 * margin, 31);
        self.footer_status
            .resize(margin + 14, footer_y + 2, w - 2 * margin - 280, 27);
        self.footer_note
            .resize(w - margin - 237, footer_y + 2, 237, 27);
    }

    fn refresh_catalog(&mut self, state: &mut State) {
        let query = self.search.value().trim().to_lowercase();
        self.search.redraw();
        state.visible = state
            .tools
            .iter()
            .enumerate()
            .filter_map(|(index, tool)| {
                let visible = (state.filter == Filter::All || tool.featured || !query.is_empty())
                    && (tool.name.to_lowercase().contains(&query)
                        || tool.path.to_string_lossy().to_lowercase().contains(&query)
                        || tool.path.canonicalize().is_ok_and(|path| {
                            path.to_string_lossy().to_lowercase().contains(&query)
                        }));
                visible.then_some(index)
            })
            .collect();
        let mut paths = std::collections::HashSet::new();
        state
            .visible
            .retain(|index| paths.insert(&state.tools[*index].path));
        if !state
            .selected
            .is_some_and(|index| state.visible.contains(&index))
        {
            state.selected = state.visible.first().copied();
        }
        state.scroll = 0;
        self.update_scroll(state);
        let count = state.visible.len();
        self.count.set_label(&format!(
            "{count} {}",
            if count == 1 {
                "tool available"
            } else {
                "tools available"
            }
        ));
        self.featured.set_value(state.filter == Filter::Featured);
        self.all.set_value(state.filter == Filter::All);
        self.list.redraw();
    }

    fn update_scroll(&mut self, state: &mut State) {
        let total_height = state.visible.len() as i32 * ROW_HEIGHT;
        let available = self.list.h().max(1);
        state.scroll = state.scroll.clamp(0, (total_height - available).max(0));
        self.scrollbar
            .set_slider_size((available as f32 / total_height.max(1) as f32).min(1.0));
        self.scrollbar
            .set_bounds(0.0, (total_height - available).max(0) as f64);
        self.scrollbar.set_value(state.scroll as f64);
        if total_height > available {
            self.scrollbar.show();
        } else {
            self.scrollbar.hide();
        }
    }

    fn refresh_selection(&mut self, state: &State) {
        if let Some(tool) = state.selected.map(|index| &state.tools[index]) {
            self.name.set_label(&tool.name);
            self.kind.set_label(if tool.mode == Mode::Desktop {
                "DESKTOP APPLICATION"
            } else {
                "COMMAND LINE TOOL"
            });
            self.tool_icon.set_label(tool.icon);
            self.path.set_label(&tool.path.to_string_lossy());
            self.path.set_tooltip(&tool.path.to_string_lossy());
            self.description.set_label(tool.description);
            self.args.set_value(tool.args);
            self.terminal.set_value(tool.mode == Mode::Terminal);
        } else {
            self.name.set_label("Select a tool");
            self.kind.set_label("");
            self.tool_icon.set_label("shell");
            self.path.set_label("");
            self.path.set_tooltip("");
            self.description
                .set_label("Filter the catalog or browse for an executable.");
            self.args.set_value("");
            self.terminal.set_value(false);
        }
        self.list.redraw();
        self.sync_controls(state);
    }

    fn sync_controls(&mut self, state: &State) {
        let selected = state.selected.is_some();
        let running = state.capture.is_some();
        enable(&mut self.run, selected && !running);
        enable(&mut self.stop, running);
        enable(&mut self.open, selected);
        enable(&mut self.args, selected);
        enable(&mut self.terminal, selected);
        enable(&mut self.root, state.elevation.is_none());
        enable(&mut self.clear, self.buffer.length() > 0);
    }

    fn act(&mut self, action: Action, state: &mut State) {
        match action {
            Action::Search | Action::Featured | Action::All => {
                if matches!(action, Action::Featured) {
                    state.filter = Filter::Featured;
                }
                if matches!(action, Action::All) {
                    state.filter = Filter::All;
                }
                let previous = state.selected;
                self.refresh_catalog(state);
                if previous != state.selected {
                    self.refresh_selection(state);
                }
            }
            Action::Scroll => {
                state.scroll = self.scrollbar.value() as i32;
                self.update_scroll(state);
                self.list.redraw();
            }
            Action::Wheel(delta) => {
                state.scroll += delta;
                self.update_scroll(state);
                self.list.redraw();
            }
            Action::Select(row) => {
                if let Some(&tool) = state.visible.get(row) {
                    if state.selected != Some(tool) {
                        state.selected = Some(tool);
                        self.refresh_selection(state);
                    }
                    let top = row as i32 * ROW_HEIGHT;
                    state.scroll = state
                        .scroll
                        .clamp((top + ROW_HEIGHT - self.list.h()).max(0), top);
                    self.update_scroll(state);
                    self.list.redraw();
                }
            }
            Action::Browse => unreachable!("the file chooser is handled outside the state borrow"),
            Action::Run | Action::Open => {
                let Some(tool) = state.selected.map(|index| &state.tools[index]) else {
                    return;
                };
                if matches!(action, Action::Run) && state.capture.is_some() {
                    return;
                }
                let argv = match shell_words::split(&self.args.value()) {
                    Ok(argv) => argv,
                    Err(error) => {
                        self.report_error(&format!("Invalid arguments. {error}."));
                        return;
                    }
                };
                if matches!(action, Action::Run) {
                    match process::start(&tool.path, &argv) {
                        Ok(capture) => {
                            self.clear_output();
                            let display = std::iter::once(tool.path.to_string_lossy().into_owned())
                                .chain(argv)
                                .collect::<Vec<_>>();
                            self.append(&format!("$ {}\n\n", shell_words::join(display)), 'C');
                            self.output_status("Running", false);
                            self.footer_status.set_label("Capture running");
                            state.capture = Some(capture);
                        }
                        Err(error) => self.report_error(&format!(
                            "Could not start {}. {error}",
                            tool.path.display()
                        )),
                    }
                } else if matches!(action, Action::Open) {
                    let result = if self.terminal.value() {
                        let args = argv.into_iter().map(OsString::from).collect::<Vec<_>>();
                        process::terminal(tool.path.as_os_str(), &args)
                    } else {
                        let mut command = Command::new(&tool.path);
                        command.args(argv);
                        process::detached(command)
                    };
                    match result {
                        Ok(()) => self
                            .footer_status
                            .set_label(&format!("Opened {}", tool.name)),
                        Err(error) => self.report_error(&format!(
                            "Could not open {}. {error}",
                            tool.path.display()
                        )),
                    }
                }
            }
            Action::Stop => {
                if let Some(capture) = &state.capture {
                    capture.stop.store(true, Ordering::Relaxed);
                    self.output_status("Stopping…", false);
                }
            }
            Action::Root => {
                if unsafe { libc::geteuid() } == 0 || state.elevation.is_some() {
                    return;
                }
                match process::elevated() {
                    Ok(child) => {
                        state.elevation = child;
                        self.footer_status
                            .set_label("Waiting for administrator authentication");
                    }
                    Err(error) => self.report_error(&format!("Could not open as root. {error}")),
                }
            }
            Action::Clear => {
                self.clear_output();
                if state.capture.is_none() {
                    self.output_status("Idle", false);
                }
            }
        }
        self.sync_controls(state);
    }

    fn browse(&mut self, state: &mut State, path: PathBuf) {
        if !catalog::is_executable(&path) {
            self.report_error(&format!(
                "Select an executable file. {} cannot be executed.",
                path.display()
            ));
            return;
        }
        let selected = state
            .tools
            .iter()
            .position(|tool| tool.path == path)
            .unwrap_or_else(|| {
                state.tools.push(catalog::browsed(path));
                state.tools.len() - 1
            });
        state.filter = Filter::All;
        self.search.set_value("");
        state.selected = Some(selected);
        self.refresh_catalog(state);
        if let Some(row) = state.visible.iter().position(|&index| index == selected) {
            state.scroll = row as i32 * ROW_HEIGHT;
            self.update_scroll(state);
        }
        self.refresh_selection(state);
    }

    fn clear_output(&mut self) {
        self.buffer.set_text("");
        self.styles.set_text("");
        self.output.hide();
        self.empty.show();
        self.output_panel.redraw();
    }

    fn append(&mut self, text: &str, style: char) {
        let text = text.replace('\0', "�");
        self.styles.append(&style.to_string().repeat(text.len()));
        self.buffer.append(&text);
        if self.buffer.length() > OUTPUT_LIMIT {
            let mut cutoff = self.buffer.length() - OUTPUT_LIMIT + 4096;
            while cutoff < self.buffer.length() && self.buffer.byte_at(cutoff) & 0xc0 == 0x80 {
                cutoff += 1;
            }
            self.buffer.remove(0, cutoff);
            self.styles.remove(0, cutoff);
        }
        self.empty.hide();
        self.output.show();
        let end = self.buffer.length();
        self.output.set_insert_position(end);
        self.output.show_insert_position();
    }

    fn output_status(&mut self, text: &str, error: bool) {
        self.output_state.set_label(text);
        self.output_state.set_label_color(if error {
            Color::from_hex(0xff9f89)
        } else {
            DIM_LIGHT
        });
    }

    fn report_error(&mut self, text: &str) {
        self.footer_status.set_label(text);
        let text = text.to_owned();
        app::add_timeout3(0.0, move |_| dialog::alert_default(&text));
    }

    fn poll(&mut self, state: &mut State) {
        if let Some(capture) = &state.capture {
            for _ in 0..64 {
                match capture.events.try_recv() {
                    Ok(process::Event::Output { text, error }) => {
                        self.append(&text, if error { 'B' } else { 'A' })
                    }
                    Ok(process::Event::Done { message, success }) => {
                        self.append(&format!("\n[{message}]\n"), if success { 'C' } else { 'B' });
                        self.output_status(&message, !success);
                        self.footer_status.set_label("Capture finished");
                        state.capture = None;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.output_status("Capture ended unexpectedly", true);
                        self.footer_status.set_label("Capture ended unexpectedly");
                        state.capture = None;
                        break;
                    }
                }
            }
            self.sync_controls(state);
        }
        if let Some(child) = &mut state.elevation {
            let result = child.try_wait();
            if !matches!(result, Ok(None)) {
                state.elevation = None;
                self.sync_controls(state);
                match result {
                    Ok(Some(status)) if status.success() => {
                        self.footer_status.set_label("Privileged window closed")
                    }
                    Ok(Some(status)) if status.code() == Some(126) => {
                        self.footer_status.set_label("Authentication canceled")
                    }
                    _ => match process::elevated_with_sudo() {
                        Ok(()) => self
                            .footer_status
                            .set_label("Waiting for administrator authentication in a terminal"),
                        Err(error) => self
                            .report_error(&format!("Could not open a privileged window. {error}")),
                    },
                }
            }
        }
    }
}

fn choose_executable() -> Option<PathBuf> {
    let mut chooser = NativeFileChooser::new(FileDialogType::BrowseFile);
    chooser.set_title("Select an executable");
    chooser.set_directory(&"/usr/bin").ok();
    chooser.show();
    let path = chooser.filename();
    (!path.as_os_str().is_empty()).then_some(path)
}

fn bind<W: WidgetExt>(widget: &mut W, sender: app::Sender<Action>, action: Action) {
    widget.set_callback(move |_| sender.send(action));
}

fn enable<W: WidgetExt>(widget: &mut W, enabled: bool) {
    if enabled && !widget.active() {
        widget.activate();
    }
    if !enabled && widget.active() {
        widget.deactivate();
    }
}

fn field_back() -> Frame {
    let mut frame = Frame::default();
    frame.draw(|f| rounded(f.x(), f.y(), f.w(), f.h(), 7, CARD, BORDER));
    frame
}

fn input() -> Input {
    let mut input = Input::default();
    input.set_frame(FrameType::FlatBox);
    input.set_color(CARD);
    input.set_text_color(INK);
    input.set_text_size(14);
    input
}

fn catalog_list(state: Rc<RefCell<State>>, sender: app::Sender<Action>) -> Frame {
    let mut frame = Frame::default();
    let drawing_state = state.clone();
    frame.draw(move |f| {
        let state = drawing_state.borrow();
        let (x, y, w, h) = (f.x(), f.y(), f.w(), f.h());
        draw::draw_rect_fill(x, y, w, h, CARD);
        draw::push_clip(x, y, w, h);
        if state.visible.is_empty() {
            draw::set_font(Font::Helvetica, 13);
            draw::set_draw_color(MUTED);
            draw::draw_text2("No matching tools", x, y + 10, w, 28, Align::Center);
        }
        let start = (state.scroll / ROW_HEIGHT) as usize;
        for (row, &index) in state
            .visible
            .iter()
            .enumerate()
            .skip(start)
            .take((h / ROW_HEIGHT + 2) as usize)
        {
            let tool = &state.tools[index];
            let y = y + row as i32 * ROW_HEIGHT - state.scroll;
            let selected = state.selected == Some(index);
            if selected {
                rounded(
                    x + 2,
                    y + 1,
                    w - 5,
                    ROW_HEIGHT - 3,
                    7,
                    TINT,
                    if f.has_focus() { RUST } else { TINT },
                );
            } else {
                line(
                    x + 8,
                    y + ROW_HEIGHT - 1,
                    x + w - 8,
                    y + ROW_HEIGHT - 1,
                    BORDER,
                );
            }
            let tile = if selected {
                Color::from_hex(0xedd4c6)
            } else {
                PAPER
            };
            rounded(x + 10, y + 8, 31, 31, 7, tile, tile);
            icon(
                tool.icon,
                x + 18,
                y + 15,
                if selected { RUST } else { MUTED },
            );
            draw::set_font(Font::HelveticaBold, 14);
            draw::set_draw_color(if selected { RUST_HOVER } else { INK });
            draw::draw_text2(
                &tool.name,
                x + 51,
                y + 6,
                w - 58,
                18,
                Align::Left | Align::Clip | Align::Inside,
            );
            draw::set_font(Font::Courier, 11);
            draw::set_draw_color(MUTED);
            draw::draw_text2(
                &tool.path.to_string_lossy(),
                x + 51,
                y + 25,
                w - 58,
                16,
                Align::Left | Align::Clip | Align::Inside,
            );
        }
        draw::pop_clip();
    });
    frame.handle(move |f, event| match event {
        Event::Push if app::event_mouse_button() == app::MouseButton::Left => {
            let row = (app::event_y() - f.y() + state.borrow().scroll) / ROW_HEIGHT;
            f.take_focus().ok();
            sender.send(Action::Select(row as usize));
            true
        }
        Event::MouseWheel => {
            let delta = match app::event_dy() {
                app::MouseWheel::Up => -ROW_HEIGHT,
                app::MouseWheel::Down => ROW_HEIGHT,
                _ => return false,
            };
            sender.send(Action::Wheel(delta));
            true
        }
        Event::Focus | Event::Unfocus => {
            f.redraw();
            true
        }
        Event::KeyDown => {
            let state = state.borrow();
            let current = state
                .visible
                .iter()
                .position(|i| Some(*i) == state.selected)
                .unwrap_or(0);
            let page = (f.h() / ROW_HEIGHT).max(1) as usize;
            let last = state.visible.len().saturating_sub(1);
            let next = match app::event_key() {
                Key::Down => (current + 1).min(last),
                Key::Up => current.saturating_sub(1),
                Key::PageDown => (current + page).min(last),
                Key::PageUp => current.saturating_sub(page),
                Key::Home => 0,
                Key::End => last,
                _ => return false,
            };
            sender.send(Action::Select(next));
            true
        }
        _ => false,
    });
    frame
}
