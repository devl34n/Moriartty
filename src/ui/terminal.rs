//! The terminal window: one compact title bar holding the tabs, and below
//! it the current tab's VTE widget, attached to a shell on a pty.
//!
//! Data flow (per tab)
//!   pty --read--> log --> shell integration marks (read only) --> vte.feed()
//!   vte "commit" (keys, paste, terminal replies) --> output queue --> pty
//!
//! Rule for the RefCells below: never call into VTE (feed, paste_text, ...)
//! while a borrow is held, because VTE can synchronously emit "commit",
//! which re-enters `send_user`.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use vte::prelude::*;

use super::config_dialog::{self, Mode};
use super::form::rgba;
use super::keys::{self, Action, Keymap};
use super::{ask, strip_controls, style};
use crate::config::*;
use crate::logging::{self, SessionLog};
use crate::marks::{Mark, Scanner};
use crate::matching::{self, PCRE2_CASELESS, PCRE2_MULTILINE, PCRE2_UTF};
use crate::paste::{is_invisible, paste_preview, sanitize_paste};
use crate::pty::{self, ExitState, Pty, ReadStatus};
use crate::storage;

struct Session {
    pty: Pty,
    log: Option<SessionLog>,
}

/// A top-level window: title bar with the tabs, find bar, and the tabs'
/// terminals in a stack. Its settings are shared by all of its tabs.
pub struct TermWindow {
    app: gtk::Application,
    window: gtk::ApplicationWindow,
    /// The menus' actions ("term.copy", ...). A private group: actions added
    /// to an ApplicationWindow itself are exported on the session bus, where
    /// any program could trigger Copy All or Paste.
    actions: gio::SimpleActionGroup,
    /// One page per tab; the visible one is the current tab.
    stack: gtk::Stack,
    /// The tabs' headers, in tab order.
    strip: gtk::Box,
    strip_scroll: gtk::ScrolledWindow,
    /// The title bar. A tab dropped on it lands where the pointer is.
    bar: gtk::Box,
    title_buttons: gtk::Box,
    /// This window's own style (title bar colours, transparency), for
    /// widgets under its `css_class`.
    css: gtk::CssProvider,
    css_class: String,
    /// The open settings window, so there is only ever one per window.
    settings_win: RefCell<Option<glib::WeakRef<gtk::ApplicationWindow>>>,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_case: gtk::CheckButton,
    /// The saved sessions, shared by every menu that lists them and read
    /// again each time one of those menus opens.
    sessions_menu: gio::Menu,
    /// The ▾ next to the + button.
    sessions_btn: gtk::MenuButton,
    settings: Rc<RefCell<Settings>>,
    /// Keyboard shortcuts, from `settings` (see `rebuild_keymap`).
    keymap: RefCell<Keymap>,
    /// In tab order, matching `strip`.
    tabs: RefCell<Vec<Rc<Tab>>>,
    force_close: Cell<bool>,
}

/// The window a tab is in. Replaced when the tab moves to another window, so
/// read it through `Tab::win`, `Tab::window` and `Tab::settings`.
struct Home {
    win: Weak<TermWindow>,
    window: gtk::ApplicationWindow,
    settings: Rc<RefCell<Settings>>,
}

impl Home {
    fn of(win: &Rc<TermWindow>) -> Self {
        Home {
            win: Rc::downgrade(win),
            window: win.window.clone(),
            settings: win.settings.clone(),
        }
    }
}

/// One terminal and the program running in it.
pub struct Tab {
    home: RefCell<Home>,
    /// The stack page (terminal + scrollbar) and its header in the strip.
    page: gtk::Box,
    header: gtk::Box,
    label: gtk::Label,
    /// In the header while the tab's output is written to a log file.
    log_icon: gtk::Image,
    term: vte::Terminal,
    scrollbar: gtk::Scrollbar,
    popover: gtk::PopoverMenu,
    /// VTE match tag of the URL regex, while clickable URLs are enabled.
    url_tag: Cell<Option<i32>>,
    hover_url: RefCell<Option<String>>,
    /// `-e` command; `None` runs the shell.
    command: Option<Vec<String>>,
    start_dir: Option<PathBuf>,
    session: RefCell<Option<Session>>,
    outq: RefCell<Vec<u8>>,
    io_src: RefCell<Option<glib::SourceId>>,
    /// Watches for the program ending (see `watch_exit`).
    exit_src: RefCell<Option<glib::SourceId>>,
    out_src: RefCell<Option<glib::SourceId>>,
    copy_src: RefCell<Option<glib::SourceId>>,
    /// A pending refresh of `auto_title`, scheduled by activity in the tab
    /// (see `schedule_title_refresh`). None while the tab is idle.
    title_src: RefCell<Option<glib::SourceId>>,
    /// The last MAX_EVENTS events.
    events: RefCell<VecDeque<String>>,
    /// The latest event and how often it has repeated since.
    last_event: RefCell<(String, u32)>,
    /// The open Event Log window, and its text, kept up to date as events
    /// arrive.
    event_win: RefCell<Option<glib::WeakRef<gtk::Window>>>,
    event_view: RefCell<Option<glib::WeakRef<gtk::TextBuffer>>>,
    last_size: Cell<(u16, u16)>,
    generation: Cell<u32>,
    bells: RefCell<VecDeque<Instant>>,
    bell_mute_until: Cell<Option<Instant>>,
    pointer: Cell<(f64, f64)>,
    /// The tab was closed: nothing may start in it any more.
    closed: Cell<bool>,
    ended: Cell<bool>,
    /// A bell rang while the tab was in the background.
    attention: Cell<bool>,
    /// Title set by the program (OSC 0/2).
    remote_title: RefCell<Option<String>>,
    /// "program: directory", used when the program sets no title.
    auto_title: RefCell<Option<String>>,
    /// Shell integration: reads the shell's marks in the output.
    marks: RefCell<Scanner>,
    /// Rows (absolute, as VTE counts them) of earlier prompts, oldest first.
    prompt_rows: RefCell<VecDeque<i64>>,
    /// A prompt is shown and not yet answered with Enter. VTE draws output
    /// after `feed` returns, so its row is read when the screen changes
    /// (`prompt_row`) and kept once Enter is pressed (`commit_prompt`).
    prompt_open: Cell<bool>,
    prompt_row: Cell<Option<i64>>,
    /// When the command now running started, as the shell reported it.
    command_started: Cell<Option<Instant>>,
}

thread_local! {
    static CHROME_CSS_LOADED: Cell<bool> = const { Cell::new(false) };
    static NEXT_WINDOW: Cell<u32> = const { Cell::new(0) };
    /// The URL regex, compiled once and shared by every tab.
    static URL_REGEX: OnceCell<Result<vte::Regex, String>> = const { OnceCell::new() };
    /// The tab being dragged by its header, if any.
    static TAB_DRAG: RefCell<Option<TabDrag>> = const { RefCell::new(None) };
}

struct TabDrag {
    tab: Weak<Tab>,
    end: DragEnd,
}

/// What happens to a dragged tab once the drag has ended.
enum DragEnd {
    /// Reordering in its own window is done live, so nothing is left to do.
    Stay,
    /// Dropped where no window takes it: into a window of its own.
    Detach,
    /// Dropped on another window, at this position or else last.
    Attach(Weak<TermWindow>, Option<usize>),
}

fn dragged_tab() -> Option<Rc<Tab>> {
    TAB_DRAG.with_borrow(|d| d.as_ref().and_then(|d| d.tab.upgrade()))
}

fn set_drag_end(end: DragEnd) {
    TAB_DRAG.with_borrow_mut(|d| {
        if let Some(d) = d {
            d.end = end;
        }
    });
}

/// What a dragged tab offers: nothing to read. The u64 can only be matched
/// inside this process (GDK has no way to turn another program's data into
/// one), so no other program, nor another Moriartty, can pose as a tab. The
/// empty private type is there because compositors only complete a drop of
/// something with a MIME type.
fn tab_drag_content() -> gdk::ContentProvider {
    gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_bytes(TAB_DRAG_MIME, &glib::Bytes::from_static(b"")),
        gdk::ContentProvider::for_value(&0u64.to_value()),
    ])
}

/// Dragging a tab by its header: `TermWindow::tab_drag_over` reorders it in
/// its own title bar, and any other window that can take it is a drop target.
/// A drop that nothing takes detaches the tab. That includes its own terminal
/// area, which is the only way out of a maximized window. Escape keeps it.
fn tab_drag_source(tab: &Rc<Tab>) -> gtk::DragSource {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    let w = Rc::downgrade(tab);
    source.connect_prepare(move |s, x, y| {
        let tab = w.upgrade()?;
        let icon = gtk::WidgetPaintable::new(Some(&tab.header));
        s.set_icon(Some(&icon), x as i32, y as i32);
        TAB_DRAG.set(Some(TabDrag {
            tab: Rc::downgrade(&tab),
            end: DragEnd::Stay,
        }));
        Some(tab_drag_content())
    });
    // True: no animation of the tab flying back to where it came from.
    source.connect_drag_cancel(|_, _, reason| {
        let detach = reason != gdk::DragCancelReason::UserCancelled;
        if detach {
            set_drag_end(DragEnd::Detach);
        }
        detach
    });
    source.connect_drag_end(|s, _, _| {
        // The icon refers to the header, which owns this controller.
        s.set_icon(None::<&gdk::Paintable>, 0, 0);
        finish_tab_drag();
    });
    source
}

/// Carry out a finished drag, from idle: moving a tab takes its header, which
/// the drag started from, out of the window, and may close that window.
fn finish_tab_drag() {
    let Some(drag) = TAB_DRAG.take() else {
        return;
    };
    glib::idle_add_local_once(move || {
        let Some(tab) = drag.tab.upgrade().filter(|t| !t.closed.get()) else {
            return;
        };
        let Some(from) = tab.win() else {
            return;
        };
        match drag.end {
            DragEnd::Stay => {}
            DragEnd::Detach => from.detach_tab(&tab),
            DragEnd::Attach(to, at) => {
                // It may have been closed while the drag was going on.
                if let Some(to) = to.upgrade().filter(|to| to.window.is_visible()) {
                    from.move_tab_to_window(&tab, &to, at);
                }
            }
        }
    });
}

fn url_regex() -> Result<vte::Regex, String> {
    URL_REGEX.with(|re| {
        re.get_or_init(|| {
            vte::Regex::for_match(
                matching::URL_REGEX,
                PCRE2_MULTILINE | PCRE2_UTF | PCRE2_CASELESS,
            )
            .map_err(|e| e.to_string())
        })
        .clone()
    })
}

/// Run when a tab's terminal area gets a new size (set once the tab exists).
type AllocateHook = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// CSS class of the widgets between the window and a terminal, which are
/// cleared when the background opacity is below 100.
const CLEAR_CLASS: &str = "moriartty-clear";
/// CSS class of the terminals, for the gap around their text.
const TERM_CLASS: &str = "moriartty-term";
/// CSS class of the title bar of a window a dragged tab can be dropped on.
const DROP_CLASS: &str = "moriartty-drop";
/// See `tab_drag_content`.
const TAB_DRAG_MIME: &str = "application/x-moriartty-tab";
const MAX_OUTQ: usize = 16 << 20;
/// Reads of what a program wrote before it ended: more than the kernel holds
/// for a terminal, and a limit, as a job it left behind can keep printing.
const MAX_DRAIN_READS: u32 = 64;
/// How long after activity in a tab its "program: directory" name is read
/// again. Long enough for a command started with Enter to have replaced
/// the shell in the foreground.
const TITLE_REFRESH: Duration = Duration::from_millis(400);
const MAX_EVENTS: usize = 1000;
/// Prompts remembered per tab for jumping between them.
const MAX_PROMPTS: usize = 10_000;
/// Copy All asks first above this many lines of scrollback.
const COPY_ALL_ASK_LINES: u64 = 1000;

/// The title bar and tabs, as one slim row (about 25px). GTK's defaults are
/// sized for touch screens, so every part that sets the bar's height gets
/// an explicit size: buttons, the window controls (and their icons, which
/// the theme pads), the tabs and their text. Symbolic icons are drawn for
/// 16px and go faint below that.
const CHROME_CSS: &str = "
.moriartty-bar {
    min-height: 0;
    padding: 0 2px;
    margin: 0;
    border-bottom: 1px solid alpha(currentColor, 0.12);
}
.moriartty-bar.moriartty-drop {
    box-shadow: inset 0 -2px alpha(currentColor, 0.55);
}
.moriartty-bar button {
    min-height: 0;
    min-width: 0;
    padding: 2px 4px;
    margin: 1px 0;
    border-radius: 4px;
}
.moriartty-bar button image,
.moriartty-bar windowcontrols image {
    -gtk-icon-size: 18px;
    padding: 0;
    margin: 0;
    min-height: 0;
    min-width: 0;
}
.moriartty-bar windowcontrols {
    margin: 0;
    border-spacing: 2px;
}
.moriartty-bar windowcontrols > button {
    min-height: 0;
    min-width: 0;
    padding: 2px;
    margin: 1px 0;
}
.moriartty-tab {
    font-size: 0.9em;
    padding: 0 1px 0 8px;
    margin: 2px 1px;
    border-radius: 4px;
    transition: background-color 120ms ease-out;
}
.moriartty-tab:hover {
    background-color: alpha(currentColor, 0.08);
}
.moriartty-tab.active {
    background-color: alpha(currentColor, 0.14);
    box-shadow: inset 0 -2px alpha(currentColor, 0.55);
}
.moriartty-tab button {
    padding: 1px;
    margin: 0;
    opacity: 0;
    transition: opacity 120ms ease-out;
}
.moriartty-tab button image {
    -gtk-icon-size: 16px;
}
.moriartty-tab:hover button,
.moriartty-tab.active button {
    opacity: 1;
}
.moriartty-tab .moriartty-log {
    color: #e01b24;
    -gtk-icon-size: 12px;
}
";

fn load_chrome_css() {
    if CHROME_CSS_LOADED.with(|c| c.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let p = gtk::CssProvider::new();
    p.load_from_string(CHROME_CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &p,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// The saved sessions, for the menus that list them. Read each time such a
/// menu opens, so sessions saved meanwhile (in any window) show up.
fn fill_sessions_menu(menu: &gio::Menu) {
    let list = gio::Menu::new();
    for name in storage::list_sessions() {
        // Menu labels take "_" as a mnemonic marker.
        let item = gio::MenuItem::new(Some(&name.replace('_', "__")), None);
        item.set_action_and_target_value(Some("term.open-session"), Some(&name.to_variant()));
        list.append_item(&item);
    }
    let manage = gio::Menu::new();
    manage.append(
        Some("_Manage Saved Sessions…"),
        Some("term.change-settings"),
    );
    menu.remove_all();
    menu.append_section(None, &list);
    menu.append_section(None, &manage);
}

fn build_menu(sessions: &gio::Menu) -> gio::Menu {
    let menu = gio::Menu::new();

    let edit = gio::Menu::new();
    edit.append(Some("_Copy"), Some("term.copy"));
    edit.append(Some("_Paste"), Some("term.paste"));
    edit.append(Some("Copy _All to Clipboard"), Some("term.copy-all"));
    edit.append(Some("_Find…"), Some("term.find"));
    menu.append_section(None, &edit);

    let zoom = gio::Menu::new();
    zoom.append(Some("Zoom _In"), Some("term.zoom-in"));
    zoom.append(Some("Zoom _Out"), Some("term.zoom-out"));
    zoom.append(Some("_Normal Size"), Some("term.zoom-reset"));
    menu.append_section(None, &zoom);

    let session = gio::Menu::new();
    session.append(Some("New _Tab"), Some("term.new-tab"));
    session.append(Some("Ne_w Window"), Some("term.new-window"));
    session.append_submenu(Some("Sa_ved Sessions"), sessions);
    session.append(Some("Clos_e Tab"), Some("term.close-tab"));
    session.append(Some("_Detach Tab"), Some("term.detach-tab"));
    session.append(Some("_Restart Shell"), Some("term.restart"));
    session.append(Some("_Settings…"), Some("term.change-settings"));
    session.append(Some("_Event Log"), Some("term.event-log"));
    menu.append_section(None, &session);

    let term = gio::Menu::new();
    term.append(Some("C_lear Scrollback"), Some("term.clear-scrollback"));
    term.append(Some("Rese_t Terminal"), Some("term.reset"));
    term.append(Some("_Full Screen"), Some("term.fullscreen"));
    menu.append_section(None, &term);

    let about = gio::Menu::new();
    about.append(Some("_About Moriartty"), Some("term.about"));
    menu.append_section(None, &about);

    menu
}

/// Hover text for a link. International host names can imitate other sites
/// letter for letter, so for those the real (ASCII) host is shown as well.
fn link_tooltip(url: &str) -> String {
    match matching::idn_ascii_host(url) {
        Some(host) => {
            format!("Ctrl+click to open {url}\n⚠ International address: the real host is {host}")
        }
        None => format!("Ctrl+click to open {url}"),
    }
}

fn css_rgb(c: Rgb) -> String {
    format!("rgb({},{},{})", c[0], c[1], c[2])
}

/// "2 min 13 s", for how long a command ran.
fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, s) => format!("{s} s"),
        (0, m, s) => format!("{m} min {s} s"),
        (h, m, _) => format!("{h} h {m} min"),
    }
}

/// A flat title bar button that leaves the keyboard focus in the terminal.
fn bar_button(icon: &str, action: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_has_frame(false);
    b.set_focus_on_click(false);
    b.set_action_name(Some(action));
    b.set_tooltip_text(Some(tooltip));
    b
}

pub fn open(
    app: &gtk::Application,
    settings: Settings,
    command: Option<Vec<String>>,
    cwd: Option<PathBuf>,
) {
    let tw = TermWindow::new(app, settings);
    let key_problems = tw.rebuild_keymap();
    let tab = tw.add_tab(command, cwd);
    // Problems found while loading the saved settings, e.g. a broken
    // Default Settings file that was moved aside, or a bad shortcut.
    for n in storage::take_notices().into_iter().chain(key_problems) {
        tab.log_event(&n);
        tab.feed_status(&n);
    }
    tw.window.present();
}

// ====================================================================== window

impl TermWindow {
    fn new(app: &gtk::Application, settings: Settings) -> Rc<Self> {
        load_chrome_css();
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .icon_name(crate::APP_ID)
            .build();
        let actions = gio::SimpleActionGroup::new();
        window.insert_action_group("term", Some(&actions));

        // The title bar is a single slim row:
        //   [window controls] [tab][tab][tab] [+] [empty] [⚙][☰] [window controls]
        // It is a WindowHandle: the empty part behaves like any title bar
        // (drag to move, double-click does what the desktop is set to do),
        // so it is deliberately not also a "new tab" area.
        let strip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .accessible_role(gtk::AccessibleRole::TabList)
            .build();
        let strip_scroll = gtk::ScrolledWindow::new();
        strip_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        strip_scroll.set_propagate_natural_width(true);
        strip_scroll.set_child(Some(&strip));
        // When the tabs don't all fit, the mouse wheel scrolls them sideways.
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let sw = strip_scroll.clone();
        wheel.connect_scroll(move |_, dx, dy| {
            let adj = sw.hadjustment();
            adj.set_value(adj.value() + (dx + dy) * 40.0);
            glib::Propagation::Stop
        });
        strip_scroll.add_controller(wheel);
        let add_btn = bar_button(
            "list-add-symbolic",
            "term.new-tab",
            "New tab (Ctrl+Shift+T)",
        );
        // ▾ next to +: open a saved session (in a new window, with all of
        // its settings: its colours and the command it runs).
        let sessions_menu = gio::Menu::new();
        fill_sessions_menu(&sessions_menu);
        let sessions_btn = gtk::MenuButton::new();
        sessions_btn.set_icon_name("pan-down-symbolic");
        sessions_btn.set_has_frame(false);
        sessions_btn.set_focus_on_click(false);
        sessions_btn.set_menu_model(Some(&sessions_menu));
        sessions_btn.set_tooltip_text(Some("Open a saved session (Ctrl+Shift+S)"));
        let filler = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        filler.set_hexpand(true);

        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_has_frame(false);
        menu_btn.set_menu_model(Some(&build_menu(&sessions_menu)));
        menu_btn.set_tooltip_text(Some("Menu (also Ctrl+Right-click)"));
        for btn in [&sessions_btn, &menu_btn] {
            if let Some(p) = btn.popover() {
                let m = sessions_menu.clone();
                p.connect_show(move |_| fill_sessions_menu(&m));
            }
        }
        let settings_btn = bar_button(
            "preferences-system-symbolic",
            "term.change-settings",
            "Settings (Ctrl+Shift+P)",
        );
        let title_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_buttons.append(&settings_btn);
        title_buttons.append(&menu_btn);

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        bar.add_css_class("moriartty-bar");
        bar.append(&gtk::WindowControls::new(gtk::PackType::Start));
        bar.append(&strip_scroll);
        bar.append(&add_btn);
        bar.append(&sessions_btn);
        bar.append(&filler);
        bar.append(&title_buttons);
        bar.append(&gtk::WindowControls::new(gtk::PackType::End));
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));
        window.set_titlebar(Some(&handle));

        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        // Find bar (Ctrl+Shift+F), hidden until used. Acts on the current tab.
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some("Find in scrollback"));
        let prev_btn = gtk::Button::from_icon_name("go-up-symbolic");
        prev_btn.set_tooltip_text(Some("Previous (older) match — Enter, Ctrl+Shift+G"));
        let next_btn = gtk::Button::from_icon_name("go-down-symbolic");
        next_btn.set_tooltip_text(Some("Next (newer) match — Ctrl+G"));
        let search_case = gtk::CheckButton::with_label("Match case");
        let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        search_box.append(&search_entry);
        search_box.append(&prev_btn);
        search_box.append(&next_btn);
        search_box.append(&search_case);
        let search_bar = gtk::SearchBar::new();
        search_bar.set_child(Some(&search_box));
        search_bar.connect_entry(&search_entry);
        search_bar.set_show_close_button(true);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.append(&search_bar);
        vbox.append(&stack);
        window.set_child(Some(&vbox));
        // Between the window and the terminal: kept see-through when the
        // background opacity is below 100 (see apply_window_settings).
        vbox.add_css_class(CLEAR_CLASS);
        stack.add_css_class(CLEAR_CLASS);

        let n = NEXT_WINDOW.with(|c| c.replace(c.get().wrapping_add(1)));
        let css_class = format!("moriartty-win-{n}");
        window.add_css_class(&css_class);
        let css = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            // Above CHROME_CSS, which it refines for this window.
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }

        let tw = Rc::new(TermWindow {
            app: app.clone(),
            window,
            actions,
            stack,
            strip,
            strip_scroll,
            bar,
            title_buttons,
            css,
            css_class,
            settings_win: RefCell::new(None),
            search_bar,
            search_entry,
            search_case,
            sessions_menu,
            sessions_btn,
            settings: Rc::new(RefCell::new(settings)),
            keymap: RefCell::new(Keymap::default()),
            tabs: RefCell::new(Vec::new()),
            force_close: Cell::new(false),
        });

        tw.apply_window_settings();
        tw.connect_signals();
        tw.connect_tab_drops();
        tw.connect_search(&prev_btn, &next_btn);
        tw.install_actions();
        let w = Rc::downgrade(&tw);
        style::on_change(move || match w.upgrade() {
            Some(tw) => {
                tw.restyle();
                true
            }
            None => false,
        });
        tw
    }

    fn apply_window_settings(&self) {
        let s = self.settings.borrow();
        self.title_buttons.set_visible(s.title_buttons);
        let c = s.active_colours(style::dark());
        let cls = &self.css_class;
        // The gap between the text and the window edge.
        let mut css = format!(
            ".{cls} .{TERM_CLASS} {{ padding: {}px; }}\n",
            s.window_border
        );
        // The title bar and the menus in the terminal's own colours, so the
        // window reads as one surface. High contrast themes keep their own.
        if style::high_contrast() {
            super::debug(format!(
                "{cls}: high contrast, title bar and menus keep the theme's colours"
            ));
        } else {
            let (bg, fg) = (css_rgb(c.background), css_rgb(c.foreground));
            super::debug(format!(
                "{cls}: title bar and menus: background {bg}, text {fg}"
            ));
            css.push_str(&format!(
                ".{cls} .moriartty-bar {{ background-color: {bg}; color: {fg}; }}\n\
                 .{cls} popover > contents, .{cls} popover > arrow {{ background-color: {bg}; color: {fg}; }}\n\
                 .{cls} popover modelbutton:hover {{ background-color: alpha(currentColor, 0.1); }}\n"
            ));
        }
        if s.background_opacity < 100 {
            // The window and every widget between it and the terminal must
            // be clear, or the terminal fades into whatever the GTK theme
            // paints there (some themes give stacks or boxes a background)
            // instead of into the desktop. Only Moriartty's own widgets,
            // marked CLEAR_CLASS: the title bar, menus and scrollbar keep
            // their look.
            css.push_str(&format!(
                "window.{cls} {{ background-color: transparent; }}\n\
                 .{cls} .{CLEAR_CLASS} {{ background: none; box-shadow: none; }}\n"
            ));
            super::debug(format!(
                "{cls}: window background transparent (opacity {}%)",
                s.background_opacity
            ));
        }
        drop(s);
        self.css.load_from_string(&css);
    }

    /// Settings or the desktop's style changed: window and all its tabs.
    fn restyle(&self) {
        super::debug(format!(
            "{}: restyle (dark: {})",
            self.css_class,
            style::dark()
        ));
        self.apply_window_settings();
        for tab in self.tabs_snapshot() {
            tab.apply_appearance();
            tab.check_size_after_layout();
        }
        super::debug(format!("{}: restyle done", self.css_class));
    }

    /// Live preview from the settings window. Only the look changes; Apply
    /// makes it stick, and closing the settings window without applying
    /// previews the old settings again.
    fn preview(&self, s: &Settings) {
        {
            let mut cur = self.settings.borrow_mut();
            cur.auto_colours = s.auto_colours;
            cur.colours = s.colours.clone();
            cur.bold_as_colour = s.bold_as_colour;
            cur.background_opacity = s.background_opacity;
        }
        self.restyle();
    }

    // ------------------------------------------------------------------ tabs

    fn add_tab(
        self: &Rc<Self>,
        command: Option<Vec<String>>,
        start_dir: Option<PathBuf>,
    ) -> Rc<Tab> {
        let tab = Tab::new(self, command, start_dir);
        self.insert_tab(&tab, None);
        glib::spawn_future_local(tab.clone().start());
        tab
    }

    /// Put a tab in this window, at position `at` or else last, and show it.
    fn insert_tab(self: &Rc<Self>, tab: &Rc<Tab>, at: Option<usize>) {
        self.tabs.borrow_mut().push(tab.clone());
        self.strip.append(&tab.header);
        self.stack.add_child(&tab.page);
        if let Some(at) = at {
            self.move_tab_to(tab, at);
        }
        self.set_action_enabled("detach-tab", self.tab_count() > 1);
        self.select_tab(tab);
    }

    /// Show a tab: its page, its header highlighted, its title on the
    /// window; then (once laid out) resize its pty and give it the focus.
    /// Pages that are not shown are not laid out, so the size is checked here.
    fn select_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        // The find bar moves on to this tab, and its highlight with it.
        if self.search_bar.is_search_mode() {
            if let Some(prev) = self.current_tab().filter(|t| !Rc::ptr_eq(t, tab)) {
                prev.clear_search();
            }
        }
        self.stack.set_visible_child(&tab.page);
        for t in self.tabs_snapshot() {
            let current = Rc::ptr_eq(&t, tab);
            if current {
                t.header.add_css_class("active");
            } else {
                t.header.remove_css_class("active");
            }
            t.header
                .update_state(&[gtk::accessible::State::Selected(Some(current))]);
        }
        tab.attention.set(false);
        tab.update_label();
        self.show_tab_chrome(tab);
        let (w, t) = (Rc::downgrade(self), Rc::downgrade(tab));
        glib::idle_add_local_once(move || {
            let (Some(win), Some(tab)) = (w.upgrade(), t.upgrade()) else {
                return;
            };
            win.reveal_tab(&tab);
            tab.check_size();
            if win.search_bar.is_search_mode() {
                win.search_update();
            } else {
                tab.term.grab_focus();
            }
        });
    }

    fn find_tab(&self, page: &gtk::Widget) -> Option<Rc<Tab>> {
        self.tabs
            .borrow()
            .iter()
            .find(|t| t.page.upcast_ref::<gtk::Widget>() == page)
            .cloned()
    }

    fn current_tab(&self) -> Option<Rc<Tab>> {
        let page = self.stack.visible_child()?;
        self.find_tab(&page)
    }

    fn is_current(&self, tab: &Tab) -> bool {
        self.stack
            .visible_child()
            .is_some_and(|c| &c == tab.page.upcast_ref::<gtk::Widget>())
    }

    fn tab_count(&self) -> usize {
        self.tabs.borrow().len()
    }

    fn tabs_snapshot(&self) -> Vec<Rc<Tab>> {
        self.tabs.borrow().clone()
    }

    /// Window title and per-tab actions follow the tab being shown.
    fn show_tab_chrome(&self, tab: &Tab) {
        self.window.set_title(Some(&tab.title()));
        self.set_action_enabled("restart", tab.ended.get());
    }

    fn tab_changed(&self, tab: &Tab) {
        if self.is_current(tab) {
            self.show_tab_chrome(tab);
        }
    }

    /// Close a tab, asking first if a program is running in it.
    fn close_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        let busy = tab.has_foreground_job();
        if busy && self.settings.borrow().warn_on_close {
            let (win, tab) = (self.clone(), tab.clone());
            glib::spawn_future_local(async move {
                let r = ask(
                    &win.window,
                    "Close this tab?",
                    "A program is still running in it and will be terminated.",
                    &["Cancel", "Close"],
                    0,
                    1,
                )
                .await;
                if r == 1 {
                    // It may have moved to another window meanwhile.
                    if let Some(win) = tab.win() {
                        win.remove_tab(&tab);
                    }
                }
            });
            return;
        }
        self.remove_tab(tab);
    }

    /// Remove a tab for good; the window closes with its last tab.
    fn remove_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        if tab.closed.replace(true) {
            return;
        }
        tab.shutdown();
        let next = self.take_tab(tab);
        tab.popover.unparent();
        self.after_tab_left(next);
    }

    /// Take a tab out of this window, leaving it running. Returns the tab to
    /// show instead, if this one was being shown; pass it to `after_tab_left`.
    fn take_tab(&self, tab: &Rc<Tab>) -> Option<Rc<Tab>> {
        let was_current = self.is_current(tab);
        let next = {
            let mut tabs = self.tabs.borrow_mut();
            let i = tabs.iter().position(|t| Rc::ptr_eq(t, tab));
            if let Some(i) = i {
                tabs.remove(i);
            }
            // The tab to its right takes its place, or else the one to its left.
            i.and_then(|i| {
                tabs.get(i)
                    .or_else(|| i.checked_sub(1).and_then(|j| tabs.get(j)))
                    .cloned()
            })
        };
        self.strip.remove(&tab.header);
        self.stack.remove(&tab.page);
        next.filter(|_| was_current)
    }

    fn after_tab_left(self: &Rc<Self>, next: Option<Rc<Tab>>) {
        self.set_action_enabled("detach-tab", self.tab_count() > 1);
        if self.tab_count() == 0 {
            self.force_close.set(true);
            self.window.close();
        } else if let Some(next) = next {
            self.select_tab(&next);
        }
    }

    /// Whether a tab may move here. Settings belong to a window, so a moved
    /// tab takes on its new window's. Only windows with the same settings
    /// take it: a production session's red background must not turn into an
    /// ordinary look, and Restart Shell must not run something else.
    fn accepts_tab(&self, tab: &Tab) -> bool {
        self.settings
            .borrow()
            .shares_tabs_with(&tab.settings().borrow())
    }

    /// Move a tab, with its running program and scrollback, to another
    /// window, at position `at` or else last. This window closes if it was
    /// the last one here.
    fn move_tab_to_window(self: &Rc<Self>, tab: &Rc<Tab>, to: &Rc<TermWindow>, at: Option<usize>) {
        tab.popover.popdown();
        // This window's find bar highlighted a match in it.
        if self.search_bar.is_search_mode() && self.is_current(tab) {
            tab.clear_search();
        }
        let next = self.take_tab(tab);
        tab.enter_window(to);
        to.insert_tab(tab, at);
        // The new window's zoom may differ.
        tab.apply_appearance();
        tab.check_size_after_layout();
        tab.log_event("Moved to another window");
        self.after_tab_left(next);
        to.window.present();
    }

    /// Give a tab a new window of its own, with this window's settings. It
    /// keeps its size: a terminal asks for the columns and rows it has.
    fn detach_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        if self.tab_count() < 2 {
            return;
        }
        let s = self.settings.borrow().clone();
        let to = TermWindow::new(&self.app, s);
        // Bad shortcuts were reported when this window loaded the same ones.
        let _ = to.rebuild_keymap();
        self.move_tab_to_window(tab, &to, None);
    }

    /// Ctrl+PgUp / Ctrl+PgDn / Ctrl+Tab, wrapping around at the ends.
    fn cycle_tab(self: &Rc<Self>, forward: bool) {
        let tabs = self.tabs_snapshot();
        let n = tabs.len();
        if n < 2 {
            return;
        }
        let cur = self
            .current_tab()
            .and_then(|c| tabs.iter().position(|t| Rc::ptr_eq(t, &c)))
            .unwrap_or(0);
        let next = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        self.select_tab(&tabs[next]);
    }

    fn tab_index(&self, tab: &Tab) -> Option<usize> {
        self.tabs
            .borrow()
            .iter()
            .position(|t| std::ptr::eq(Rc::as_ptr(t), tab))
    }

    /// Ctrl+Shift+PgUp / Ctrl+Shift+PgDn.
    fn move_tab(&self, tab: &Tab, forward: bool) {
        let Some(i) = self.tab_index(tab) else { return };
        let to = match (forward, i.checked_sub(1)) {
            (true, _) => i + 1,
            (false, Some(j)) => j,
            (false, None) => return,
        };
        self.move_tab_to(tab, to);
    }

    /// Put a tab at position `to` (counted as if it were not in the list).
    fn move_tab_to(&self, tab: &Tab, to: usize) {
        let order = {
            let mut tabs = self.tabs.borrow_mut();
            let Some(i) = tabs.iter().position(|t| std::ptr::eq(Rc::as_ptr(t), tab)) else {
                return;
            };
            if i == to || to >= tabs.len() {
                return;
            }
            let t = tabs.remove(i);
            tabs.insert(to, t);
            tabs.clone()
        };
        let mut prev: Option<gtk::Widget> = None;
        for t in &order {
            self.strip.reorder_child_after(&t.header, prev.as_ref());
            prev = Some(t.header.clone().upcast());
        }
    }

    /// Where a dragged tab goes when the pointer is at `x`, in the strip's
    /// coordinates: after every other tab whose middle is to the left of it.
    fn drop_index(&self, tab: &Tab, x: f64) -> usize {
        self.tabs_snapshot()
            .iter()
            .filter(|t| !std::ptr::eq(Rc::as_ptr(t), tab))
            .filter(|t| {
                t.header
                    .compute_bounds(&self.strip)
                    .is_some_and(|b| f64::from(b.x() + b.width() / 2.0) < x)
            })
            .count()
    }

    /// Where a point in the window is along the tab strip, if it is on the
    /// title bar.
    fn strip_x_at(&self, x: f64, y: f64) -> Option<f64> {
        let point = graphene::Point::new(x as f32, y as f32);
        let on_bar = self.window.compute_point(&self.bar, &point)?;
        if !self
            .bar
            .contains(f64::from(on_bar.x()), f64::from(on_bar.y()))
        {
            return None;
        }
        self.window
            .compute_point(&self.strip, &point)
            .map(|p| f64::from(p.x()))
    }

    /// A dragged tab is over this window, at (`x`, `y`): reorder it if this
    /// is its window, and say whether it can be dropped here.
    fn tab_drag_over(&self, x: f64, y: f64) -> gdk::DragAction {
        let Some(tab) = dragged_tab() else {
            return gdk::DragAction::empty();
        };
        if self.tab_index(&tab).is_none() {
            if !self.accepts_tab(&tab) {
                return gdk::DragAction::empty();
            }
            self.bar.add_css_class(DROP_CLASS);
            return gdk::DragAction::MOVE;
        }
        match self.strip_x_at(x, y) {
            Some(x) => {
                self.move_tab_to(&tab, self.drop_index(&tab, x));
                gdk::DragAction::MOVE
            }
            // Over its own terminal: refused, so a drop here detaches it.
            None => gdk::DragAction::empty(),
        }
    }

    /// A dragged tab was dropped on this window, anywhere in it. The move
    /// waits until the drag has ended (`finish_tab_drag`).
    fn tab_dropped(self: &Rc<Self>, x: f64, y: f64) -> bool {
        self.bar.remove_css_class(DROP_CLASS);
        let Some(tab) = dragged_tab() else {
            return false;
        };
        if self.tab_index(&tab).is_some() {
            // Already in place: tab_drag_over moved it along the way.
            return self.strip_x_at(x, y).is_some();
        }
        if !self.accepts_tab(&tab) {
            return false;
        }
        let at = self.strip_x_at(x, y).map(|x| self.drop_index(&tab, x));
        set_drag_end(DragEnd::Attach(Rc::downgrade(self), at));
        true
    }

    /// The whole window takes dragged tabs, not only the title bar: another
    /// window's terminal is an easy place to drop one. Only tabs from this
    /// process match (see `tab_drag_content`).
    fn connect_tab_drops(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(glib::Type::U64, gdk::DragAction::MOVE);
        let w = Rc::downgrade(self);
        target.connect_enter(move |_, x, y| {
            w.upgrade()
                .map_or(gdk::DragAction::empty(), |tw| tw.tab_drag_over(x, y))
        });
        let w = Rc::downgrade(self);
        target.connect_motion(move |_, x, y| {
            w.upgrade()
                .map_or(gdk::DragAction::empty(), |tw| tw.tab_drag_over(x, y))
        });
        let w = Rc::downgrade(self);
        target.connect_leave(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.bar.remove_css_class(DROP_CLASS);
            }
        });
        let w = Rc::downgrade(self);
        target.connect_drop(move |_, _, x, y| w.upgrade().is_some_and(|tw| tw.tab_dropped(x, y)));
        self.window.add_controller(target);
    }

    /// Scroll the tab strip so the tab's header is fully visible.
    fn reveal_tab(&self, tab: &Tab) {
        let Some(b) = tab.header.compute_bounds(&self.strip) else {
            return;
        };
        let adj = self.strip_scroll.hadjustment();
        let (x, w) = (f64::from(b.x()), f64::from(b.width()));
        if x < adj.value() {
            adj.set_value(x);
        } else if x + w > adj.value() + adj.page_size() {
            adj.set_value(x + w - adj.page_size());
        }
    }

    /// Rebuild the keyboard shortcuts from the settings. Returns the
    /// problems found; bad entries are skipped, never fatal.
    fn rebuild_keymap(&self) -> Vec<String> {
        let (map, problems) = {
            let s = self.settings.borrow();
            Keymap::new(&s.keys, s.fullscreen_on_alt_enter)
        };
        *self.keymap.borrow_mut() = map;
        problems
    }

    /// The action for a key press. A layout without Latin letters reports
    /// Ctrl+Shift+T as Ctrl+Shift+Tau, say, so a key that isn't ASCII also
    /// counts as the Latin key in the same place.
    fn action_for(&self, key: gdk::Key, keycode: u32, state: gdk::ModifierType) -> Option<Action> {
        let keymap = self.keymap.borrow();
        keymap.lookup(key, state).or_else(|| {
            if !keys::is_non_ascii(key) {
                return None;
            }
            let levels = WidgetExt::display(&self.window).map_keycode(keycode)?;
            let latin = keys::latin_equivalent(levels.iter().map(|(k, v)| (k.level(), *v)))?;
            keymap.lookup(latin, state)
        })
    }

    /// Tab and window shortcuts. Handled for the whole window, so they work
    /// wherever the keyboard focus is (terminal, find bar, ...). The
    /// switching keys are only taken with two or more tabs, so in a
    /// single-tab window Ctrl+PgUp, Ctrl+Tab and Alt+digit still reach
    /// programs like vim and readline. The terminal's own shortcuts (copy,
    /// zoom, ...) are left to the tab.
    fn tab_shortcut(
        self: &Rc<Self>,
        key: gdk::Key,
        keycode: u32,
        state: gdk::ModifierType,
    ) -> bool {
        let Some(action) = self.action_for(key, keycode, state) else {
            return false;
        };
        let many_tabs = self.tab_count() > 1;
        match action {
            Action::NewTab => self.new_tab(),
            Action::CloseTab => {
                if let Some(tab) = self.current_tab() {
                    self.close_tab(&tab);
                }
            }
            Action::NewWindow => self.new_window(),
            Action::Sessions => self.sessions_btn.popup(),
            Action::PreviousTab
            | Action::NextTab
            | Action::MoveTabLeft
            | Action::MoveTabRight
            | Action::DetachTab
            | Action::GoToTab(_)
                if !many_tabs =>
            {
                return false
            }
            Action::PreviousTab => self.cycle_tab(false),
            Action::NextTab => self.cycle_tab(true),
            Action::MoveTabLeft | Action::MoveTabRight => {
                if let Some(tab) = self.current_tab() {
                    self.move_tab(&tab, action == Action::MoveTabRight);
                }
            }
            Action::DetachTab => {
                if let Some(tab) = self.current_tab() {
                    self.detach_tab(&tab);
                }
            }
            Action::GoToTab(n) => match self.tabs_snapshot().get(n) {
                Some(tab) => self.select_tab(tab),
                None => return false,
            },
            _ => return false,
        }
        true
    }

    // --------------------------------------------------------------- signals

    fn connect_signals(self: &Rc<Self>) {
        // Capture phase on the window: runs before the focused widget sees
        // the key, whichever widget that is.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, keycode, state| match w.upgrade() {
            Some(tw) if tw.tab_shortcut(key, keycode, state) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);

        // Warn before closing while programs are running. This closure holds
        // the strong reference that keeps the TermWindow alive for the
        // window's lifetime.
        let strong = self.clone();
        self.window.connect_close_request(move |win| {
            let tw = &strong;
            let tabs = tw.tabs_snapshot();
            let busy = tabs.iter().filter(|t| t.has_foreground_job()).count();
            if !tw.force_close.get() && busy > 0 && tw.settings.borrow().warn_on_close {
                let detail = if tabs.len() == 1 {
                    "A program is still running in it and will be terminated.".to_string()
                } else if busy == 1 {
                    "A program is still running in one of its tabs and will be terminated."
                        .to_string()
                } else {
                    format!(
                        "Programs are still running in {busy} of its tabs and will be terminated."
                    )
                };
                let tw2 = tw.clone();
                let win = win.clone();
                glib::spawn_future_local(async move {
                    let r = ask(
                        &win,
                        "Close this window?",
                        &detail,
                        &["Cancel", "Close"],
                        0,
                        1,
                    )
                    .await;
                    if r == 1 {
                        tw2.force_close.set(true);
                        win.close();
                    }
                });
                return glib::Propagation::Stop;
            }
            let tabs: Vec<Rc<Tab>> = tw.tabs.borrow_mut().drain(..).collect();
            for t in &tabs {
                t.closed.set(true);
                t.shutdown();
                t.popover.unparent();
            }
            glib::Propagation::Proceed
        });

        let w = Rc::downgrade(self);
        let css = self.css.clone();
        self.window.connect_destroy(move |win| {
            gtk::style_context_remove_provider_for_display(&WidgetExt::display(win), &css);
            if let Some(tw) = w.upgrade() {
                for t in tw.tabs_snapshot() {
                    t.popover.unparent();
                }
            }
        });
    }

    /// A signal handler that calls `f` with this window, while it exists.
    fn with_window<T: 'static>(
        self: &Rc<Self>,
        f: impl Fn(&Rc<TermWindow>) + 'static,
    ) -> impl Fn(&T) + 'static {
        let w = Rc::downgrade(self);
        move |_: &T| {
            if let Some(tw) = w.upgrade() {
                f(&tw);
            }
        }
    }

    fn connect_search(self: &Rc<Self>, prev_btn: &gtk::Button, next_btn: &gtk::Button) {
        self.search_entry
            .connect_search_changed(self.with_window(|tw| tw.search_update()));
        self.search_case
            .connect_toggled(self.with_window(|tw| tw.search_update()));
        // Enter and the up button go to older output (what you usually want
        // in a terminal); Ctrl+G / the down button go back towards newer.
        self.search_entry
            .connect_activate(self.with_window(|tw| tw.search_find(true)));
        self.search_entry
            .connect_previous_match(self.with_window(|tw| tw.search_find(true)));
        self.search_entry
            .connect_next_match(self.with_window(|tw| tw.search_find(false)));
        prev_btn.connect_clicked(self.with_window(|tw| tw.search_find(true)));
        next_btn.connect_clicked(self.with_window(|tw| tw.search_find(false)));
        // Closing the bar (Escape or ×) clears the highlight and returns focus.
        let w = Rc::downgrade(self);
        self.search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                if bar.is_search_mode() {
                    return;
                }
                let Some(tab) = w.upgrade().and_then(|tw| tw.current_tab()) else {
                    return;
                };
                tab.clear_search();
                tab.term.grab_focus();
            });
    }

    fn show_search(&self) {
        self.search_bar.set_search_mode(true);
        self.search_entry.grab_focus();
        self.search_entry.select_region(0, -1);
    }

    fn search_update(&self) {
        let Some(tab) = self.current_tab() else {
            return;
        };
        let text = self.search_entry.text();
        self.search_entry.remove_css_class("error");
        if text.is_empty() {
            tab.term.search_set_regex(None, 0);
            return;
        }
        let mut flags = PCRE2_MULTILINE | PCRE2_UTF;
        if !self.search_case.is_active() {
            flags |= PCRE2_CASELESS;
        }
        match vte::Regex::for_search(&matching::regex_escape(&text), flags) {
            Ok(re) => {
                tab.term.search_set_regex(Some(&re), 0);
                tab.term.search_set_wrap_around(true);
                tab.term.unselect_all();
                self.search_find(true);
            }
            Err(_) => self.search_entry.add_css_class("error"),
        }
    }

    /// `older == true` searches upwards into the scrollback.
    fn search_find(&self, older: bool) {
        if self.search_entry.text().is_empty() {
            return;
        }
        let Some(tab) = self.current_tab() else {
            return;
        };
        let found = if older {
            tab.term.search_find_previous()
        } else {
            tab.term.search_find_next()
        };
        if found {
            self.search_entry.remove_css_class("error");
        } else {
            self.search_entry.add_css_class("error");
        }
    }

    /// Font zoom: 1 bigger, -1 smaller, 0 back to normal. Stored in the
    /// window's settings, so it applies to all its tabs, New Window inherits
    /// it and the settings dialog can save it as the default.
    fn zoom(&self, step: i32) {
        let zoom = {
            let mut s = self.settings.borrow_mut();
            s.font_zoom = next_zoom(s.font_zoom, step);
            s.font_zoom
        };
        for tab in self.tabs_snapshot() {
            tab.term.set_font_scale(f64::from(zoom) / 100.0);
            tab.check_size_after_layout();
        }
    }

    // --------------------------------------------------------------- actions

    fn add_action(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<TermWindow>) + 'static) {
        let a = gio::SimpleAction::new(name, None);
        let w = Rc::downgrade(self);
        a.connect_activate(move |_, _| {
            if let Some(tw) = w.upgrade() {
                f(&tw);
            }
        });
        self.actions.add_action(&a);
    }

    /// An action that works on the tab currently shown.
    fn add_tab_action(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Tab>) + 'static) {
        self.add_action(name, move |tw| {
            if let Some(tab) = tw.current_tab() {
                f(&tab);
            }
        });
    }

    fn set_action_enabled(&self, name: &str, enabled: bool) {
        if let Some(a) = self
            .actions
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
        {
            a.set_enabled(enabled);
        }
    }

    fn install_actions(self: &Rc<Self>) {
        self.add_tab_action("copy", |t| t.copy());
        self.add_tab_action("paste", |t| t.paste(false));
        self.add_tab_action("copy-all", |t| t.copy_all());
        self.add_tab_action("clear-scrollback", |t| {
            let n = t.settings().borrow().scrollback;
            t.term.set_scrollback_lines(0);
            t.term.set_scrollback_lines(i64::from(n));
        });
        self.add_tab_action("reset", |t| {
            t.term.reset(true, false);
            t.forget_prompts();
        });
        // A saved session from the sessions menu; the name is the target.
        let open_session = gio::SimpleAction::new("open-session", Some(glib::VariantTy::STRING));
        let w = Rc::downgrade(self);
        open_session.connect_activate(move |_, name| {
            if let (Some(tw), Some(name)) = (w.upgrade(), name.and_then(|v| v.get::<String>())) {
                tw.open_session(&name);
            }
        });
        self.actions.add_action(&open_session);
        self.add_tab_action("event-log", |t| t.show_event_log());
        self.add_tab_action("restart", |t| t.restart());
        self.add_action("find", |tw| tw.show_search());
        self.add_action("zoom-in", |tw| tw.zoom(1));
        self.add_action("zoom-out", |tw| tw.zoom(-1));
        self.add_action("zoom-reset", |tw| tw.zoom(0));
        self.add_action("fullscreen", |tw| tw.toggle_fullscreen());
        self.add_action("new-window", |tw| tw.new_window());
        self.add_action("new-tab", |tw| tw.new_tab());
        self.add_action("close-tab", |tw| {
            if let Some(tab) = tw.current_tab() {
                tw.close_tab(&tab);
            }
        });
        self.add_action("detach-tab", |tw| {
            if let Some(tab) = tw.current_tab() {
                tw.detach_tab(&tab);
            }
        });
        self.add_action("about", |tw| {
            gtk::AboutDialog::builder()
                .transient_for(&tw.window)
                .modal(true)
                .program_name("Moriartty")
                .logo_icon_name(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("A fast, private-by-default terminal inspired by PuTTY's pterm")
                // Not license_type, which adds "See the ... for details".
                .copyright("This program comes with absolutely no warranty.")
                .build()
                .present();
        });
        self.add_action("change-settings", |tw| {
            // One settings window per terminal window. One that is no
            // longer shown has gone (destroyed, or on its way): never
            // present that again, open a new one instead.
            let stored = tw.settings_win.borrow().as_ref().and_then(|w| w.upgrade());
            if let Some(win) = stored.filter(|w| w.is_visible()) {
                win.present();
                return;
            }
            let (wa, wp) = (Rc::downgrade(tw), Rc::downgrade(tw));
            let s = tw.settings.borrow().clone();
            let win = config_dialog::show(
                &tw.app,
                s,
                Mode::Reconfigure {
                    apply: Rc::new(move |new| {
                        if let Some(tw) = wa.upgrade() {
                            tw.reconfigure(new);
                        }
                    }),
                    preview: Rc::new(move |s: &Settings| {
                        if let Some(tw) = wp.upgrade() {
                            tw.preview(s);
                        }
                    }),
                },
            );
            // Forget it once destroyed: a destroyed window can linger in
            // memory, and presenting it again leaves a dead window that
            // no button closes.
            let w = Rc::downgrade(tw);
            win.connect_destroy(move |_| {
                if let Some(tw) = w.upgrade() {
                    tw.settings_win.borrow_mut().take();
                }
            });
            *tw.settings_win.borrow_mut() = Some(win.downgrade());
        });
        self.set_action_enabled("restart", false);
    }

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    fn current_cwd(&self) -> Option<PathBuf> {
        self.current_tab().and_then(|t| t.cwd())
    }

    fn new_window(&self) {
        let s = self.settings.borrow().clone();
        open(&self.app, s, None, self.current_cwd());
    }

    fn new_tab(self: &Rc<Self>) {
        let cwd = self.current_cwd();
        self.add_tab(None, cwd);
    }

    /// A saved session, in a new window: settings belong to a window, so
    /// the session's colours and command apply in full, as in PuTTY.
    fn open_session(&self, name: &str) {
        match storage::load_session(name) {
            Ok(s) => open(&self.app, s, None, self.current_cwd()),
            Err(e) => super::message(&self.window, "Could not open the session", &e),
        }
    }

    fn reconfigure(&self, new: Settings) {
        *self.settings.borrow_mut() = new;
        self.apply_window_settings();
        let key_problems = self.rebuild_keymap();
        for tab in self.tabs_snapshot() {
            tab.apply_appearance();
            tab.check_size_after_layout();
            tab.update_title();
            tab.log_event("Settings changed (shell, command and logging changes apply on restart)");
            for p in &key_problems {
                tab.log_event(p);
            }
        }
    }
}

// ========================================================================= tab

impl Tab {
    fn new(
        win: &Rc<TermWindow>,
        command: Option<Vec<String>>,
        start_dir: Option<PathBuf>,
    ) -> Rc<Self> {
        let (columns, rows) = {
            let s = win.settings.borrow();
            (s.columns, s.rows)
        };
        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_size(i64::from(columns), i64::from(rows));
        term.add_css_class(TERM_CLASS);

        let scrollbar =
            gtk::Scrollbar::new(gtk::Orientation::Vertical, term.vadjustment().as_ref());

        // Resize hook without polling: the terminal is overlaid on an
        // invisible DrawingArea that always gets exactly the same space, and
        // DrawingArea emits "resize" whenever that space changes (window
        // resize, maximise, find bar shown, ...). The terminal itself is laid
        // out right after, so the check runs once GTK has finished the layout
        // pass. Font and zoom changes call check_size() directly, and so does
        // switching to the tab.
        let on_allocate: AllocateHook = Rc::default();
        let size_probe = gtk::DrawingArea::new();
        size_probe.set_can_target(false);
        size_probe.set_hexpand(true);
        size_probe.set_vexpand(true);
        let term_holder = gtk::Overlay::new();
        term_holder.set_child(Some(&size_probe));
        term_holder.add_overlay(&term);
        // The overlay's size request comes from the terminal (its columns and
        // rows), not from the empty DrawingArea.
        term_holder.set_measure_overlay(&term, true);
        {
            let on_allocate = on_allocate.clone();
            size_probe.connect_resize(move |_, _, _| {
                let on_allocate = on_allocate.clone();
                glib::idle_add_local_once(move || {
                    if let Some(f) = on_allocate.borrow().as_ref() {
                        f();
                    }
                });
            });
        }

        let page = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        page.append(&term_holder);
        page.append(&scrollbar);
        // Kept see-through with background opacity (the scrollbar is not).
        for w in [
            page.upcast_ref::<gtk::Widget>(),
            term_holder.upcast_ref(),
            size_probe.upcast_ref(),
            term.upcast_ref(),
        ] {
            w.add_css_class(CLEAR_CLASS);
        }

        // Header in the title bar: the tab's name and a close button.
        // Click selects, middle-click closes.
        let label = gtk::Label::new(Some("Moriartty"));
        label.set_ellipsize(pango::EllipsizeMode::Middle);
        label.set_width_chars(8);
        label.set_max_width_chars(28);
        let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        close_btn.set_has_frame(false);
        close_btn.set_focus_on_click(false);
        close_btn.set_tooltip_text(Some("Close tab (Ctrl+Shift+W, or middle-click the tab)"));
        close_btn.update_property(&[gtk::accessible::Property::Label("Close tab")]);
        let log_icon = gtk::Image::from_icon_name("media-record-symbolic");
        log_icon.add_css_class("moriartty-log");
        log_icon.set_visible(false);
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .accessible_role(gtk::AccessibleRole::Tab)
            .build();
        header.add_css_class("moriartty-tab");
        header.append(&log_icon);
        header.append(&label);
        header.append(&close_btn);

        let popover = gtk::PopoverMenu::from_model(Some(&build_menu(&win.sessions_menu)));
        popover.set_parent(&term);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);

        let tab = Rc::new(Tab {
            home: RefCell::new(Home::of(win)),
            page,
            header,
            label,
            log_icon,
            term,
            scrollbar,
            popover,
            url_tag: Cell::new(None),
            hover_url: RefCell::new(None),
            command,
            start_dir,
            session: RefCell::new(None),
            outq: RefCell::new(Vec::new()),
            io_src: RefCell::new(None),
            exit_src: RefCell::new(None),
            out_src: RefCell::new(None),
            copy_src: RefCell::new(None),
            title_src: RefCell::new(None),
            events: RefCell::new(VecDeque::new()),
            last_event: RefCell::new((String::new(), 0)),
            event_win: RefCell::new(None),
            event_view: RefCell::new(None),
            last_size: Cell::new((0, 0)),
            generation: Cell::new(0),
            bells: RefCell::new(VecDeque::new()),
            bell_mute_until: Cell::new(None),
            pointer: Cell::new((0.0, 0.0)),
            closed: Cell::new(false),
            ended: Cell::new(false),
            attention: Cell::new(false),
            remote_title: RefCell::new(None),
            auto_title: RefCell::new(None),
            marks: RefCell::new(Scanner::default()),
            prompt_rows: RefCell::new(VecDeque::new()),
            prompt_open: Cell::new(false),
            prompt_row: Cell::new(None),
            command_started: Cell::new(None),
        });
        tab.apply_appearance();
        tab.update_title();
        tab.connect_signals();
        tab.connect_title_signal();

        let w = Rc::downgrade(&tab);
        close_btn.connect_clicked(move |_| {
            let Some(tab) = w.upgrade() else { return };
            if let Some(win) = tab.win() {
                win.close_tab(&tab);
            }
        });
        // Presses on the rest of the header: the left button selects (and
        // dragging moves the tab, see tab_drag_source), the middle one
        // closes. Claimed, so they don't drag the window. Presses on the
        // close button are left alone: claiming them here would cancel the
        // button's own click, and the × would do nothing.
        let close_ref = close_btn.downgrade();
        let on_close = move |header: &gtk::Box, x: f64, y: f64| match (
            header.pick(x, y, gtk::PickFlags::DEFAULT),
            close_ref.upgrade(),
        ) {
            (Some(hit), Some(close)) => {
                hit == *close.upcast_ref::<gtk::Widget>() || hit.is_ancestor(&close)
            }
            _ => false,
        };
        let on_close = Rc::new(on_close);

        let click = gtk::GestureClick::new();
        click.set_button(2);
        let (w, oc) = (Rc::downgrade(&tab), on_close.clone());
        click.connect_pressed(move |g, _, x, y| {
            let Some(tab) = w.upgrade() else { return };
            let Some(win) = tab.win() else { return };
            if !oc(&tab.header, x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                win.close_tab(&tab);
            }
        });
        tab.header.add_controller(click);

        let press = gtk::GestureDrag::new();
        let (w, oc) = (Rc::downgrade(&tab), on_close);
        press.connect_drag_begin(move |g, x, y| {
            let Some(tab) = w.upgrade() else { return };
            let Some(win) = tab.win() else { return };
            if oc(&tab.header, x, y) {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            win.select_tab(&tab);
        });
        tab.header.add_controller(press.clone());
        // In one group, so claiming the press leaves the drag source working
        // (it starts by itself once the pointer has moved far enough) and
        // refusing it refuses the drag too.
        let source = tab_drag_source(&tab);
        tab.header.add_controller(source.clone());
        source.group_with(&press);

        let w = Rc::downgrade(&tab);
        *on_allocate.borrow_mut() = Some(Box::new(move || {
            if let Some(tab) = w.upgrade() {
                tab.check_size();
            }
        }));
        tab
    }

    // ------------------------------------------------------------------ home

    fn win(&self) -> Option<Rc<TermWindow>> {
        self.home.borrow().win.upgrade()
    }

    fn window(&self) -> gtk::ApplicationWindow {
        self.home.borrow().window.clone()
    }

    fn settings(&self) -> Rc<RefCell<Settings>> {
        self.home.borrow().settings.clone()
    }

    /// This tab now belongs to `win`. The context menu lists the new
    /// window's saved sessions; its actions follow the widget tree by
    /// themselves.
    fn enter_window(&self, win: &Rc<TermWindow>) {
        *self.home.borrow_mut() = Home::of(win);
        self.popover
            .set_menu_model(Some(&build_menu(&win.sessions_menu)));
    }

    // ----------------------------------------------------------------- setup

    fn apply_appearance(&self) {
        // A copy, so no borrow is held while VTE runs (it can call back
        // into this tab, see the rule at the top of this file).
        let s = self.settings().borrow().clone();
        let t = &self.term;
        t.set_font(Some(&pango::FontDescription::from_string(&s.font)));
        let c = s.active_colours(style::dark());
        let palette: Vec<gdk::RGBA> = c.ansi.iter().map(|x| rgba(*x)).collect();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        let bg = rgba(c.background);
        let alpha = s.background_opacity.clamp(30, 100) as f32 / 100.0;
        let bg = gdk::RGBA::new(bg.red(), bg.green(), bg.blue(), alpha);
        t.set_colors(Some(&rgba(c.foreground)), Some(&bg), &palette_refs);
        t.set_color_bold(Some(&rgba(c.bold_foreground)));
        t.set_color_cursor(Some(&rgba(c.cursor)));
        t.set_color_cursor_foreground(Some(&rgba(c.cursor_text)));
        t.set_bold_is_bright(s.bold_as_colour);
        t.set_cursor_shape(match s.cursor {
            CursorType::Block => vte::CursorShape::Block,
            CursorType::Underline => vte::CursorShape::Underline,
            CursorType::Vertical => vte::CursorShape::Ibeam,
        });
        t.set_cursor_blink_mode(if s.cursor_blink {
            vte::CursorBlinkMode::On
        } else {
            vte::CursorBlinkMode::Off
        });
        t.set_text_blink_mode(if s.blink_text {
            vte::TextBlinkMode::Always
        } else {
            vte::TextBlinkMode::Never
        });
        t.set_scrollback_lines(i64::from(s.scrollback));
        t.set_scroll_on_output(s.scroll_on_output);
        t.set_scroll_on_keystroke(s.scroll_on_key);
        t.set_mouse_autohide(s.hide_mouse);
        // We implement the bell ourselves (styles + overload protection).
        t.set_audible_bell(false);
        t.set_allow_hyperlink(s.allow_hyperlinks);
        t.set_word_char_exceptions(&s.word_chars);
        t.set_cjk_ambiguous_width(if s.cjk_ambig_wide { 2 } else { 1 });
        t.set_backspace_binding(match s.backspace {
            BackspaceKey::ControlH => vte::EraseBinding::AsciiBackspace,
            BackspaceKey::ControlQuestion => vte::EraseBinding::AsciiDelete,
        });
        self.scrollbar.set_visible(s.scrollbar);
        t.set_font_scale(f64::from(s.font_zoom) / 100.0);
        self.set_url_matching(s.clickable_urls);
    }

    /// Highlight http(s) URLs (pointer cursor on hover) so Ctrl+click can
    /// open them.
    fn set_url_matching(&self, enabled: bool) {
        match (enabled, self.url_tag.get()) {
            (true, None) => match url_regex() {
                Ok(re) => {
                    let tag = self.term.match_add_regex(&re, 0);
                    self.term.match_set_cursor_name(tag, "pointer");
                    self.url_tag.set(Some(tag));
                }
                Err(e) => self.log_event(format!("URL matching unavailable: {e}")),
            },
            (false, Some(tag)) => {
                self.term.match_remove(tag);
                self.url_tag.set(None);
            }
            _ => {}
        }
    }

    /// The safe http(s) link under widget coordinates (x, y), if any: a
    /// matched URL, or an OSC 8 hyperlink when those are allowed. The flag
    /// is true for a hyperlink, whose visible text the program chose and
    /// which may say something other than where it goes.
    fn link_at(&self, x: f64, y: f64) -> Option<(String, bool)> {
        if let Some(tag) = self.url_tag.get() {
            let (m, t) = self.term.check_match_at(x, y);
            if t == tag {
                if let Some(url) = m.and_then(|m| matching::safe_url(&m)) {
                    return Some((url, false));
                }
            }
        }
        if self.settings().borrow().allow_hyperlinks {
            if let Some(url) = self
                .term
                .check_hyperlink_at(x, y)
                .and_then(|h| matching::safe_url(&h))
            {
                return Some((url, true));
            }
        }
        None
    }

    fn update_link_tooltip(&self, x: f64, y: f64) {
        let link = self.link_at(x, y).map(|(url, _)| url);
        if *self.hover_url.borrow() == link {
            return;
        }
        self.term
            .set_tooltip_text(link.as_deref().map(link_tooltip).as_deref());
        *self.hover_url.borrow_mut() = link;
    }

    /// Ctrl+click on a link. A URL shown as plain ASCII text opens at once:
    /// what you see is where it goes. A hyperlink, or an international host
    /// name that may imitate another site, asks first and shows the real
    /// address.
    fn open_url(self: &Rc<Self>, url: String, hyperlink: bool) {
        let idn = matching::idn_ascii_host(&url);
        if !hyperlink && idn.is_none() {
            self.launch_url(&url, None);
            return;
        }
        let tab = self.clone();
        glib::spawn_future_local(async move {
            let mut detail = url.clone();
            if let Some(host) = &idn {
                detail.push_str(&format!(
                    "\n\n⚠ International address: the real host is {host}. \
                     It may imitate another site."
                ));
            }
            if hyperlink {
                detail.push_str(
                    "\n\nThe program chose this link's text, which may not match where it goes.",
                );
            }
            let r = ask(
                &tab.window(),
                "Open this link?",
                &detail,
                &["Cancel", "Open"],
                0,
                0,
            )
            .await;
            if r == 1 && !tab.closed.get() {
                tab.launch_url(&url, idn.as_deref());
            }
        });
    }

    fn launch_url(&self, url: &str, idn: Option<&str>) {
        match idn {
            Some(host) => self.log_event(format!("Opening {url} (real host {host})")),
            None => self.log_event(format!("Opening {url}")),
        }
        // Straight to the desktop's default browser through GIO. GTK's
        // UriLauncher may go through the OpenURI portal, which refuses
        // non-dumpable processes like Moriartty (see style.rs), and it
        // failed silently. Here every failure is reported.
        let scheme = if url
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("https"))
        {
            "https"
        } else {
            "http"
        };
        let ctx = WidgetExt::display(&self.window()).app_launch_context();
        let result = match gio::AppInfo::default_for_uri_scheme(scheme) {
            Some(app) => {
                super::debug(format!("link: opening with {}", app.name()));
                app.launch_uris(&[url], Some(&ctx))
                    .map_err(|e| e.to_string())
            }
            None => Err(format!("No application is set to open {scheme} links.")),
        };
        if let Err(e) = result {
            self.log_event(format!("Could not open {url}: {e}"));
            super::message(&self.window(), "Could not open the link", &e);
        }
    }

    /// The tab's name: the title the program set (unless that is turned
    /// off), else the -title / Window title setting, else "program:
    /// directory" like kitty.
    fn title(&self) -> String {
        let settings = self.settings();
        let s = settings.borrow();
        let remote = self
            .remote_title
            .borrow()
            .clone()
            .filter(|_| !s.no_remote_wintitle);
        let base = remote
            .or_else(|| (!s.wintitle.is_empty()).then(|| strip_controls(&s.wintitle, 200)))
            .or_else(|| self.auto_title.borrow().clone())
            .unwrap_or_else(|| "Moriartty".to_string());
        if self.ended.get() {
            format!("{base} (inactive)")
        } else {
            base
        }
    }

    /// Header text; bold while a bell is waiting to be seen.
    fn update_label(&self) {
        let title = self.title();
        let text = glib::markup_escape_text(&title);
        if self.attention.get() {
            self.label.set_markup(&format!("<b>{text}</b>"));
        } else {
            self.label.set_markup(&text);
        }
        self.header.set_tooltip_text(Some(&title));
        self.header
            .update_property(&[gtk::accessible::Property::Label(&title)]);
    }

    fn update_title(&self) {
        self.update_label();
        if let Some(win) = self.win() {
            win.tab_changed(self);
        }
    }

    /// Re-read "program: directory" once, shortly. Nothing signals a change
    /// of foreground program, but one only happens around activity: Enter
    /// starts a command, and a command ending makes the shell print its
    /// prompt. So output, Enter and the shell's own directory reports (OSC 7)
    /// schedule this, and an idle tab never wakes up. During continuous
    /// output it runs at most once per TITLE_REFRESH.
    fn schedule_title_refresh(self: &Rc<Self>) {
        if self.ended.get() || self.title_src.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(TITLE_REFRESH, move || {
            if let Some(tab) = w.upgrade() {
                // Fired: the source is gone, so shutdown() mustn't remove it.
                tab.title_src.borrow_mut().take();
                tab.refresh_auto_title();
            }
        });
        *self.title_src.borrow_mut() = Some(id);
    }

    /// Read "program: directory" from /proc now.
    fn refresh_auto_title(&self) {
        let t = {
            let g = self.session.borrow();
            g.as_ref()
                .and_then(|s| s.pty.describe())
                .map(|t| strip_controls(&t, 200))
        };
        if *self.auto_title.borrow() != t {
            *self.auto_title.borrow_mut() = t;
            self.update_title();
        }
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);

        // Everything the terminal wants to send: keystrokes, pastes, replies.
        let w = weak.clone();
        self.term.connect_commit(move |_, text, _| {
            if let Some(tab) = w.upgrade() {
                tab.send_user(text.as_bytes());
            }
        });

        // VTE puts every selection in PRIMARY itself. Copying it to the
        // clipboard too is opt-in (debounced, so dragging doesn't spam the
        // clipboard).
        let w = weak.clone();
        self.term.connect_selection_changed(move |_| {
            let Some(tab) = w.upgrade() else { return };
            if !tab.settings().borrow().copy_to_clipboard {
                return;
            }
            if let Some(id) = tab.copy_src.borrow_mut().take() {
                id.remove();
            }
            let w2 = Rc::downgrade(&tab);
            let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                if let Some(tab) = w2.upgrade() {
                    tab.copy_src.borrow_mut().take();
                    if tab.term.has_selection() {
                        tab.term.copy_clipboard_format(vte::Format::Text);
                    }
                }
            });
            *tab.copy_src.borrow_mut() = Some(id);
        });

        let w = weak.clone();
        self.term.connect_bell(move |_| {
            if let Some(tab) = w.upgrade() {
                tab.bell();
            }
        });

        // VTE has drawn more output: while a prompt is shown, that is where
        // the prompt is.
        let w = weak.clone();
        self.term.connect_contents_changed(move |t| {
            if let Some(tab) = w.upgrade() {
                if tab.prompt_open.get() {
                    tab.prompt_row.set(Some(t.cursor_position().1));
                }
            }
        });

        // Pointer position, for placing the context menu and link tooltips.
        let motion = gtk::EventControllerMotion::new();
        let w = weak.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(tab) = w.upgrade() {
                tab.pointer.set((x, y));
                tab.update_link_tooltip(x, y);
            }
        });
        self.term.add_controller(motion);

        // Ctrl+click on a link opens it in the browser. Plain clicks are
        // left alone so selecting text works as usual.
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = weak.clone();
        click.connect_pressed(move |g, _, x, y| {
            let Some(tab) = w.upgrade() else { return };
            if !g
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                return;
            }
            if let Some((url, hyperlink)) = tab.link_at(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                tab.open_url(url, hyperlink);
            }
        });
        self.term.add_controller(click);

        // Mouse buttons. A capture-phase legacy controller sees presses
        // before VTE does, so right/middle clicks can be taken over.
        // Middle-click is intercepted too, so PRIMARY pastes are filtered
        // like any other paste. Other Shift+clicks pass through to VTE and
        // the program; Shift+Middle-click can't, as VTE would paste by itself.
        let legacy = gtk::EventControllerLegacy::new();
        legacy.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = weak.clone();
        legacy.connect_event(move |_, ev| {
            let Some(tab) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if ev.event_type() != gdk::EventType::ButtonPress {
                return glib::Propagation::Proceed;
            }
            let Some(be) = ev.downcast_ref::<gdk::ButtonEvent>() else {
                return glib::Propagation::Proceed;
            };
            let mods = ev.modifier_state();
            if mods.contains(gdk::ModifierType::SHIFT_MASK) && be.button() != 2 {
                return glib::Propagation::Proceed;
            }
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let (action, to_clipboard) = {
                let settings = tab.settings();
                let s = settings.borrow();
                (s.mouse_action, s.copy_to_clipboard)
            };
            match (be.button(), ctrl) {
                (3, true) => tab.show_menu(),
                (3, false) => match action {
                    // Right-click pastes what selecting copied: PRIMARY, or
                    // the clipboard when selections are copied there too.
                    MouseAction::Compromise => tab.paste(!to_clipboard),
                    MouseAction::Windows => tab.show_menu(),
                    MouseAction::Xterm => return glib::Propagation::Proceed,
                },
                (2, _) => tab.paste(true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.term.add_controller(legacy);

        // Terminal shortcuts (copy, paste, find, zoom, prompts, ... see
        // keys::ACTIONS). Tab and window shortcuts are handled by the window
        // (TermWindow::tab_shortcut), which sees every key first.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = weak.clone();
        keys.connect_key_pressed(move |_, key, keycode, state| {
            let Some(tab) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Some(win) = tab.win() else {
                return glib::Propagation::Proceed;
            };
            let handled = match win.action_for(key, keycode, state) {
                Some(Action::Copy) => {
                    tab.copy();
                    true
                }
                Some(Action::Paste) => {
                    tab.paste(false);
                    true
                }
                Some(Action::Settings) => {
                    let _ = WidgetExt::activate_action(&tab.window(), "term.change-settings", None);
                    true
                }
                Some(Action::Find) => {
                    win.show_search();
                    true
                }
                Some(Action::ZoomIn) => {
                    win.zoom(1);
                    true
                }
                Some(Action::ZoomOut) => {
                    win.zoom(-1);
                    true
                }
                Some(Action::ZoomReset) => {
                    win.zoom(0);
                    true
                }
                Some(Action::Fullscreen) => {
                    win.toggle_fullscreen();
                    true
                }
                // Without shell integration there are no prompts to jump
                // to, and the key goes to the program.
                Some(Action::PreviousPrompt) => tab.jump_to_prompt(true),
                Some(Action::NextPrompt) => tab.jump_to_prompt(false),
                _ => false,
            };
            if handled || keys::is_vte_clipboard_key(key, state) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.term.add_controller(keys);
    }

    #[allow(deprecated)]
    fn connect_title_signal(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.term.connect_window_title_changed(move |t| {
            let Some(tab) = w.upgrade() else { return };
            let title = t.window_title().map(|s| strip_controls(&s, 200));
            *tab.remote_title.borrow_mut() = title.filter(|s| !s.trim().is_empty());
            tab.update_title();
        });
        // The shell reports a new directory (OSC 7, from VTE's shell
        // integration). Only a cue to look: the directory shown and used for
        // New Tab is always read from the shell itself (/proc), because any
        // program's output can claim to be OSC 7.
        let w = Rc::downgrade(self);
        self.term.connect_current_directory_uri_changed(move |_| {
            if let Some(tab) = w.upgrade() {
                tab.schedule_title_refresh();
            }
        });
    }

    /// Font/zoom changes alter the column/row count without changing the
    /// widget's size, so the resize hook doesn't fire for them.
    fn check_size_after_layout(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(tab) = w.upgrade() {
                tab.check_size();
            }
        });
    }

    // -------------------------------------------------------------- session

    /// Add to the Event Log. A message repeating back to back (say,
    /// "Output queue full") is counted rather than logged again.
    fn log_event(&self, msg: impl AsRef<str>) {
        let msg = strip_controls(msg.as_ref(), 4000);
        let repeats = {
            let mut last = self.last_event.borrow_mut();
            if last.0 == msg {
                last.1 = last.1.saturating_add(1);
                return;
            }
            std::mem::replace(&mut *last, (msg.clone(), 0)).1
        };
        if repeats > 0 {
            let s = if repeats == 1 { "" } else { "s" };
            self.push_event(&format!(
                "(previous message repeated {repeats} more time{s})"
            ));
        }
        self.push_event(&msg);
    }

    /// Append one line, keeping only the last MAX_EVENTS.
    fn push_event(&self, msg: &str) {
        let ts = glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%Y-%m-%d %H:%M:%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let line = format!("{ts}\t{msg}");
        let view = self.event_view.borrow().as_ref().and_then(|w| w.upgrade());
        if let Some(buf) = view {
            let sep = if buf.char_count() > 0 { "\n" } else { "" };
            buf.insert(&mut buf.end_iter(), &format!("{sep}{line}"));
            if buf.line_count() > MAX_EVENTS as i32 {
                if let Some(mut second) = buf.iter_at_line(1) {
                    buf.delete(&mut buf.start_iter(), &mut second);
                }
            }
        }
        let mut events = self.events.borrow_mut();
        events.push_back(line);
        if events.len() > MAX_EVENTS {
            events.pop_front();
        }
    }

    /// Print a status line into the terminal itself.
    fn feed_status(&self, msg: &str) {
        let line = format!("\x1b[0m\r\n[moriartty] {}\r\n", strip_controls(msg, 1000));
        self.term.feed(line.as_bytes());
    }

    fn term_size(&self) -> (u16, u16) {
        let c = self.term.column_count().clamp(1, i64::from(u16::MAX)) as u16;
        let r = self.term.row_count().clamp(1, i64::from(u16::MAX)) as u16;
        (c, r)
    }

    async fn prepare_log(&self, s: &Settings) -> Result<Option<(PathBuf, std::fs::File)>, String> {
        let path = logging::expand_path(&s.log_file)?;
        let exists = std::fs::symlink_metadata(&path).is_ok();
        let append = if !exists {
            false
        } else {
            match s.log_exists {
                LogExists::Overwrite => false,
                LogExists::Append => true,
                LogExists::Ask => {
                    let r = ask(
                        &self.window(),
                        "The session log file already exists",
                        &format!(
                            "{}\n\nOverwrite it, append to it, or disable logging for this tab?",
                            path.display()
                        ),
                        &["Disable logging", "Overwrite", "Append"],
                        0,
                        2,
                    )
                    .await;
                    match r {
                        1 => false,
                        2 => true,
                        _ => return Ok(None),
                    }
                }
            }
        };
        let f = logging::open_log(&path, append)
            .map_err(|e| format!("Cannot open log file {}: {e}", path.display()))?;
        Ok(Some((path, f)))
    }

    async fn start(self: Rc<Self>) {
        // Repairs a copy: this run falls back on safe values, but the window
        // keeps the real ones. A program missing today mustn't be forgotten
        // when Apply saves these settings.
        let mut s = self.settings().borrow().clone();
        for n in s.repair() {
            self.log_event(&n);
            self.feed_status(&n);
        }
        let gen = self.generation.get().wrapping_add(1);
        self.generation.set(gen);

        let mut log = None;
        let mut log_path = None;
        if s.log_type != LogType::None {
            match self.prepare_log(&s).await {
                Ok(Some((path, file))) => {
                    self.log_event(format!("Logging to {}", path.display()));
                    match SessionLog::new(file, s.log_type, s.log_flush, s.log_header) {
                        Ok(l) => {
                            log = Some(l);
                            log_path = Some(path);
                        }
                        Err(e) => self.log_event(format!("Logging disabled: {e}")),
                    }
                }
                Ok(None) => self.log_event("Logging disabled for this tab"),
                Err(e) => {
                    self.log_event(&e);
                    self.feed_status(&e);
                }
            }
        }
        if self.generation.get() != gen || self.closed.get() {
            return; // closed or restarted while the log dialog was open
        }

        let (cols, rows) = self.term_size();
        // `-e` wins over the saved session's command, which wins over the shell.
        let command = self.command.clone().or_else(|| s.session_command());
        let spawned = pty::spawn(
            &s,
            command.as_deref(),
            self.start_dir.as_deref(),
            cols,
            rows,
        );
        match spawned {
            Ok((pty, desc)) => {
                self.log_event(desc);
                let fd = pty.raw_fd();
                let exit_fd = pty.exit_fd();
                *self.session.borrow_mut() = Some(Session { pty, log });
                self.show_logging(log_path.as_deref());
                let w = Rc::downgrade(&self);
                let id = crate::sys::unix_fd_add_local(
                    fd,
                    glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
                    // Low, like VTE's own processing: feed only queues, so
                    // reading ahead of what VTE and GTK can show would freeze
                    // the window and pile up memory.
                    glib::Priority::DEFAULT_IDLE,
                    move |_, _| match w.upgrade() {
                        Some(tab) => tab.on_readable(),
                        None => glib::ControlFlow::Break,
                    },
                );
                if id.is_none() {
                    self.log_event("Cannot watch the terminal for output");
                }
                *self.io_src.borrow_mut() = id;
                *self.exit_src.borrow_mut() = exit_fd.and_then(|fd| self.watch_exit(fd));
                // The pty was created at this size; later changes arrive
                // through the layout hook (check_size).
                self.last_size.set((cols, rows));
                if self.win().is_some_and(|w| w.is_current(&self)) {
                    self.term.grab_focus();
                }
                // Name the tab once the program has had a moment to start;
                // after that, activity keeps the name current.
                self.schedule_title_refresh();
            }
            Err(e) => {
                self.log_event(&e);
                self.feed_status(&e);
                self.ended.set(true);
                self.update_title();
            }
        }
    }

    fn check_size(&self) {
        let size = self.term_size();
        let last = self.last_size.replace(size);
        if size == last {
            return;
        }
        // VTE rewraps the lines for a new width, which moves every row.
        if size.0 != last.0 {
            self.forget_prompts();
        }
        if let Some(sess) = self.session.borrow_mut().as_mut() {
            sess.pty.resize(size.0, size.1);
        }
    }

    /// Notice the program ending. End-of-file on the terminal isn't enough: a
    /// job left in the background (`firefox &`, then `exit`) keeps it open.
    fn watch_exit(self: &Rc<Self>, fd: RawFd) -> Option<glib::SourceId> {
        let w = Rc::downgrade(self);
        crate::sys::unix_fd_add_local(
            fd,
            glib::IOCondition::IN,
            // Like the output, which goes first (see `program_ended`).
            glib::Priority::DEFAULT_IDLE,
            move |_, _| {
                if let Some(tab) = w.upgrade() {
                    // Fired: the source goes away, so shutdown() mustn't remove it.
                    tab.exit_src.borrow_mut().take();
                    tab.program_ended();
                }
                glib::ControlFlow::Break
            },
        )
    }

    /// The program has ended and end-of-file may never come: take what it
    /// wrote before that, then finish.
    fn program_ended(self: &Rc<Self>) {
        for _ in 0..MAX_DRAIN_READS {
            if !matches!(self.read_output(), Ok(ReadStatus::More)) {
                break;
            }
        }
        self.poll_exit(None, 0);
    }

    /// Read one piece of output, for the log and for VTE.
    fn read_output(self: &Rc<Self>) -> io::Result<ReadStatus> {
        let mut data = Vec::new();
        let mut log_error = None;
        let status = {
            let mut g = self.session.borrow_mut();
            let Some(sess) = g.as_mut() else {
                return Ok(ReadStatus::Eof);
            };
            let status = sess.pty.read(&mut data);
            if !data.is_empty() {
                if let Some(log) = sess.log.as_mut() {
                    if let Err(e) = log.output(&data) {
                        sess.log = None;
                        log_error = Some(e);
                    }
                }
            }
            status
        };
        if let Some(e) = log_error {
            self.log_event(format!("Logging stopped: {e}"));
            self.show_logging(None);
        }
        if !data.is_empty() {
            self.feed_output(&data);
            self.schedule_title_refresh();
        }
        status
    }

    fn on_readable(self: &Rc<Self>) -> glib::ControlFlow {
        match self.read_output() {
            Ok(ReadStatus::More | ReadStatus::Empty) => glib::ControlFlow::Continue,
            Ok(ReadStatus::Eof) => {
                self.io_src.borrow_mut().take();
                self.poll_exit(None, 0);
                glib::ControlFlow::Break
            }
            Err(e) => {
                self.io_src.borrow_mut().take();
                self.poll_exit(Some(e.to_string()), 0);
                glib::ControlFlow::Break
            }
        }
    }

    fn send_user(self: &Rc<Self>, bytes: &[u8]) {
        {
            let g = self.session.borrow();
            if g.is_none() {
                return;
            }
        }
        let overflow = {
            let mut q = self.outq.borrow_mut();
            if q.len() + bytes.len() > MAX_OUTQ {
                true
            } else {
                q.extend_from_slice(bytes);
                false
            }
        };
        if overflow {
            self.log_event("Output queue full; input discarded");
        }
        self.flush();
        // Enter may start a program: the tab's name may be about to change,
        // and the prompt it answers is where it is now.
        if bytes.contains(&b'\r') {
            self.commit_prompt();
            self.schedule_title_refresh();
        }
    }

    // ------------------------------------------------------ shell integration

    /// Hand the program's output to VTE, acting on the shell's marks in it
    /// (see `marks`) as they come.
    fn feed_output(&self, data: &[u8]) {
        let found = self.marks.borrow_mut().scan(data);
        let mut done = 0;
        for (end, mark) in found {
            self.term.feed(&data[done..end]);
            done = end;
            self.on_mark(mark);
        }
        self.term.feed(&data[done..]);
    }

    fn on_mark(&self, mark: Mark) {
        match mark {
            Mark::Prompt => {
                self.commit_prompt();
                self.prompt_open.set(true);
            }
            Mark::CommandStart => {
                self.commit_prompt();
                self.command_started.set(Some(Instant::now()));
            }
            // Only after a CommandStart: an empty Enter, or a mark some
            // program printed on its own, is no command.
            Mark::CommandEnd(status) => {
                if let Some(t0) = self.command_started.take() {
                    self.command_finished(t0.elapsed(), status);
                }
            }
        }
    }

    /// Keep the row of the prompt being shown, if VTE has drawn it.
    fn commit_prompt(&self) {
        if !self.prompt_open.replace(false) {
            return;
        }
        let Some(row) = self.prompt_row.take() else {
            return;
        };
        let mut rows = self.prompt_rows.borrow_mut();
        // Rows only grow; after a reset they start again at the top.
        while rows.back().is_some_and(|&r| r >= row) {
            rows.pop_back();
        }
        rows.push_back(row);
        if rows.len() > MAX_PROMPTS {
            rows.pop_front();
        }
    }

    /// The rows are no longer right (reset, restart, new width).
    fn forget_prompts(&self) {
        self.prompt_rows.borrow_mut().clear();
        self.prompt_open.set(false);
        self.prompt_row.set(None);
    }

    /// Scroll to the previous (`older`) or next prompt. False when there is
    /// none to go to, so the key goes to the program instead.
    fn jump_to_prompt(&self, older: bool) -> bool {
        let Some(adj) = self.term.vadjustment() else {
            return false;
        };
        let (lower, top) = (adj.lower(), adj.value());
        let bottom = (adj.upper() - adj.page_size()).max(lower);
        let target = {
            let mut rows = self.prompt_rows.borrow_mut();
            // Rows that left the scrollback are gone.
            while rows.front().is_some_and(|&r| (r as f64) < lower) {
                rows.pop_front();
            }
            if rows.is_empty() {
                return false;
            }
            // One line above the prompt stays in view: the end of the
            // output before it, or the first line of a two-line prompt.
            let at = |r: i64| ((r - 1) as f64).clamp(lower, bottom);
            if older {
                rows.iter().rev().map(|&r| at(r)).find(|&p| p < top - 0.5)
            } else {
                rows.iter()
                    .map(|&r| at(r))
                    .find(|&p| p > top + 0.5)
                    .or((top < bottom - 0.5).then_some(bottom))
            }
        };
        match target {
            Some(p) => {
                adj.set_value(p);
                true
            }
            None => false,
        }
    }

    /// A command the shell reported has ended. If it ran long and the tab
    /// is not in front, say so: the tab's name turns bold, and the desktop
    /// shows a notice. The notice names the tab by number and gives the
    /// exit status, never the command, its output or the tab's title.
    fn command_finished(&self, took: Duration, status: Option<i32>) {
        let secs = self.settings().borrow().notify_command_secs;
        if secs == 0 || took < Duration::from_secs(u64::from(secs)) {
            return;
        }
        let Some(win) = self.win() else {
            return;
        };
        let current = win.is_current(self);
        if current && self.window().is_active() {
            return;
        }
        if !current && !self.attention.get() {
            self.attention.set(true);
            self.update_label();
        }
        let n = win.tab_index(self).map_or(1, |i| i + 1);
        let how = match status {
            Some(0) => "succeeded".to_string(),
            Some(c) => format!("failed with status {c}"),
            None => "finished".to_string(),
        };
        let body = format!("A command in tab {n} {how} after {}.", duration_text(took));
        self.log_event(&body);
        // Notifications go over D-Bus; without a connection there is none.
        if win.app.dbus_connection().is_some() {
            let note = gio::Notification::new("Command finished");
            note.set_body(Some(&body));
            win.app.send_notification(Some("command-finished"), &note);
        }
    }

    fn flush(self: &Rc<Self>) {
        let mut error = None;
        {
            let mut g = self.session.borrow_mut();
            let Some(sess) = g.as_mut() else {
                self.outq.borrow_mut().clear();
                return;
            };
            let mut q = self.outq.borrow_mut();
            while !q.is_empty() {
                match sess.pty.write(&q) {
                    Ok(0) => break,
                    Ok(n) => {
                        q.drain(..n);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        error = Some(e);
                        q.clear();
                        break;
                    }
                }
            }
            if !q.is_empty() && self.out_src.borrow().is_none() {
                let w = Rc::downgrade(self);
                let id = crate::sys::unix_fd_add_local(
                    sess.pty.raw_fd(),
                    glib::IOCondition::OUT,
                    glib::Priority::DEFAULT,
                    move |_, _| {
                        if let Some(tab) = w.upgrade() {
                            tab.out_src.borrow_mut().take();
                            tab.flush();
                        }
                        glib::ControlFlow::Break
                    },
                );
                *self.out_src.borrow_mut() = id;
            }
        }
        if let Some(e) = error {
            self.log_event(format!("Write error: {e}"));
        }
    }

    fn poll_exit(self: &Rc<Self>, err: Option<String>, attempt: u32) {
        let state = match self.session.borrow_mut().as_mut() {
            Some(s) => s.pty.exit_state(),
            None => ExitState::Gone,
        };
        if state == ExitState::Running && attempt < 30 {
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(100), move || {
                if let Some(tab) = w.upgrade() {
                    tab.poll_exit(err, attempt + 1);
                }
            });
            return;
        }
        self.finish(state, err);
    }

    fn finish(self: &Rc<Self>, state: ExitState, err: Option<String>) {
        if self.ended.get() {
            return;
        }
        self.shutdown();
        let (msg, clean) = match (&err, state) {
            (Some(e), _) => (format!("Terminal error: {e}"), false),
            (None, ExitState::Exited(0)) => ("Process exited (status 0)".to_string(), true),
            (None, ExitState::Exited(c)) => (format!("Process exited with status {c}"), false),
            (None, ExitState::Signaled(sig)) => (format!("Process killed by signal {sig}"), false),
            (None, ExitState::Gone) => ("Process ended".to_string(), true),
            (None, ExitState::Running) => {
                ("Terminal closed; process terminated".to_string(), false)
            }
        };
        self.log_event(&msg);
        let close = match self.settings().borrow().close_on_exit {
            CloseOnExit::Always => true,
            CloseOnExit::Never => false,
            CloseOnExit::Clean => clean,
        };
        if close {
            if let Some(win) = self.win() {
                win.remove_tab(self);
            }
            return;
        }
        self.feed_status(&format!(
            "{msg}. Ctrl+Right-click → Restart Shell to start again."
        ));
        self.update_title();
    }

    /// Stop all I/O and drop the pty (which hangs up the child).
    fn shutdown(&self) {
        self.ended.set(true);
        for src in [
            &self.io_src,
            &self.exit_src,
            &self.out_src,
            &self.copy_src,
            &self.title_src,
        ] {
            if let Some(id) = src.borrow_mut().take() {
                id.remove();
            }
        }
        let sess = self.session.borrow_mut().take();
        drop(sess);
        self.outq.borrow_mut().clear();
        self.show_logging(None);
    }

    /// The header's logging indicator: whether this tab's output is being
    /// recorded, and where, is always visible.
    fn show_logging(&self, path: Option<&Path>) {
        self.log_icon.set_visible(path.is_some());
        let tip = path.map(|p| format!("This tab's output is logged to {}", p.display()));
        self.log_icon.set_tooltip_text(tip.as_deref());
    }

    fn restart(self: &Rc<Self>) {
        if !self.ended.get() || self.closed.get() {
            return;
        }
        self.ended.set(false);
        self.remote_title.borrow_mut().take();
        self.auto_title.borrow_mut().take();
        self.update_title();
        self.term.reset(true, false);
        self.forget_prompts();
        *self.marks.borrow_mut() = Scanner::default();
        self.command_started.set(None);
        glib::spawn_future_local(self.clone().start());
    }

    fn has_foreground_job(&self) -> bool {
        self.session
            .borrow()
            .as_ref()
            .is_some_and(|s| s.pty.has_foreground_job())
    }

    /// The shell's current directory, for New Tab / New Window.
    fn cwd(&self) -> Option<PathBuf> {
        self.session.borrow().as_ref().and_then(|s| s.pty.cwd())
    }

    // ------------------------------------------------------------ clipboard

    fn copy(&self) {
        if self.term.has_selection() {
            self.term.copy_clipboard_format(vte::Format::Text);
        }
    }

    /// Copy All: the whole scrollback to the clipboard. Scrollback can hold
    /// secrets and clipboard managers keep what reaches the clipboard, so a
    /// large one asks first.
    fn copy_all(self: &Rc<Self>) {
        // VTE's adjustment counts rows: scrollback plus screen.
        let lines = self.term.vadjustment().map_or(0.0, |a| a.upper()) as u64;
        if lines <= COPY_ALL_ASK_LINES {
            self.copy_all_now();
            return;
        }
        let tab = self.clone();
        glib::spawn_future_local(async move {
            let r = ask(
                &tab.window(),
                &format!("Copy all {lines} lines to the clipboard?"),
                "The scrollback may contain passwords or other secrets, and clipboard \
                 managers keep a copy of everything that is copied.",
                &["Cancel", "Copy"],
                0,
                1,
            )
            .await;
            if r == 1 && !tab.closed.get() {
                tab.copy_all_now();
            }
        });
    }

    fn copy_all_now(&self) {
        self.term.select_all();
        self.term.copy_clipboard_format(vte::Format::Text);
        self.term.unselect_all();
    }

    fn paste(self: &Rc<Self>, primary: bool) {
        let clip = if primary {
            self.term.primary_clipboard()
        } else {
            self.term.clipboard()
        };
        // The paste belongs to this tab and the shell running in it now. If
        // the shell is restarted or the tab closed while the clipboard is
        // read or the dialog is open, drop the paste.
        let gen = self.generation.get();
        let tab = self.clone();
        glib::spawn_future_local(async move {
            let text = match clip.read_text_future().await {
                Ok(Some(t)) => t.to_string(),
                _ => return,
            };
            if tab.closed.get() {
                return;
            }
            let (allow_controls, confirm) = {
                let settings = tab.settings();
                let s = settings.borrow();
                (s.paste_controls, s.confirm_multiline_paste)
            };
            let clean = sanitize_paste(&text, allow_controls);
            if clean.is_empty() {
                return;
            }
            let breaks = clean.matches('\r').count();
            // Invisible or direction-changing characters can make even one
            // line run something other than what it shows.
            let hidden = clean.chars().any(is_invisible);
            if confirm && (breaks > 0 || hidden) {
                let question = if breaks > 0 {
                    let lines = breaks + usize::from(!clean.ends_with('\r'));
                    let s = if lines == 1 { "" } else { "s" };
                    format!("Paste {lines} line{s} into the terminal?")
                } else {
                    "Paste text containing hidden characters?".to_string()
                };
                // Cancel is the default: an Enter pressed out of habit
                // right after pasting must not confirm it.
                let r = ask(
                    &tab.window(),
                    &question,
                    &paste_preview(&clean, 400),
                    &["Cancel", "Paste"],
                    0,
                    0,
                )
                .await;
                if r != 1 {
                    return;
                }
            }
            let alive =
                !tab.closed.get() && tab.generation.get() == gen && tab.session.borrow().is_some();
            if alive {
                // paste_text honours bracketed-paste mode.
                tab.term.paste_text(&clean);
            }
        });
    }

    // ---------------------------------------------------------------- misc

    /// Take away what the find bar highlighted.
    fn clear_search(&self) {
        self.term.search_set_regex(None, 0);
        self.term.unselect_all();
    }

    fn bell(&self) {
        let (style, overload, n, t, mute) = {
            let settings = self.settings();
            let s = settings.borrow();
            (
                s.bell,
                s.bell_overload,
                s.bell_overload_n,
                s.bell_overload_t,
                s.bell_overload_s,
            )
        };
        // Mark background tabs, whatever the bell style.
        if !self.win().is_some_and(|w| w.is_current(self)) && !self.attention.get() {
            self.attention.set(true);
            self.update_label();
        }
        let now = Instant::now();
        if let Some(until) = self.bell_mute_until.get() {
            if now < until {
                return;
            }
            self.bell_mute_until.set(None);
        }
        if overload {
            let mut b = self.bells.borrow_mut();
            b.push_back(now);
            let window = Duration::from_secs(u64::from(t));
            while b.front().is_some_and(|f| now.duration_since(*f) > window) {
                b.pop_front();
            }
            if b.len() > n as usize {
                b.clear();
                drop(b);
                self.bell_mute_until
                    .set(Some(now + Duration::from_secs(u64::from(mute))));
                self.log_event("Bell overload: bell silenced temporarily");
                return;
            }
        }
        match style {
            BellStyle::None => {}
            BellStyle::Audible => self.term.display().beep(),
            BellStyle::Visual => {
                self.term.set_opacity(0.55);
                let term = self.term.clone();
                glib::timeout_add_local_once(Duration::from_millis(90), move || {
                    term.set_opacity(1.0)
                });
            }
        }
    }

    fn show_menu(&self) {
        if let Some(win) = self.win() {
            fill_sessions_menu(&win.sessions_menu);
        }
        let (x, y) = self.pointer.get();
        self.popover
            .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.popover.popup();
    }

    fn show_event_log(&self) {
        // One per tab: only the newest window is kept up to date.
        let open = self.event_win.borrow().as_ref().and_then(|w| w.upgrade());
        if let Some(win) = open.filter(|w| w.is_visible()) {
            win.present();
            return;
        }
        let win = gtk::Window::builder()
            .title(format!("Moriartty Event Log — {}", self.title()))
            .transient_for(&self.window())
            .default_width(720)
            .default_height(380)
            .build();
        let buf = gtk::TextBuffer::new(None);
        let text = self
            .events
            .borrow()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        buf.set_text(&text);
        *self.event_view.borrow_mut() = Some(buf.downgrade());
        let tv = gtk::TextView::with_buffer(&buf);
        tv.set_editable(false);
        tv.set_monospace(true);
        tv.set_wrap_mode(gtk::WrapMode::WordChar);
        let sw = gtk::ScrolledWindow::new();
        sw.set_child(Some(&tv));
        win.set_child(Some(&sw));
        *self.event_win.borrow_mut() = Some(win.downgrade());
        win.present();
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration_text(Duration::from_secs(12)), "12 s");
        assert_eq!(duration_text(Duration::from_secs(133)), "2 min 13 s");
        assert_eq!(
            duration_text(Duration::from_secs(3 * 3600 + 125)),
            "3 h 2 min"
        );
    }
}
