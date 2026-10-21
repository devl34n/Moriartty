//! The terminal window: one compact title bar holding the tabs, and below
//! it the current tab's VTE widget, attached to a shell on a pty.
//!
//! Data flow (per tab)
//!   pty --read--> log --> vte.feed()
//!   vte "commit" (keys, paste, terminal replies) --> output queue --> pty
//!
//! Rule for the RefCells below: never call into VTE (feed, paste_text, ...)
//! while a borrow is held, because VTE can synchronously emit "commit",
//! which re-enters `send_user`.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use vte::prelude::*;

use super::config_dialog::{self, Mode};
use super::form::rgba;
use super::{ask, strip_controls, style};
use crate::config::*;
use crate::logging::{self, SessionLog};
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
    /// One page per tab; the visible one is the current tab.
    stack: gtk::Stack,
    /// The tabs' headers, in tab order.
    strip: gtk::Box,
    strip_scroll: gtk::ScrolledWindow,
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
    settings: Rc<RefCell<Settings>>,
    /// In tab order, matching `strip`.
    tabs: RefCell<Vec<Rc<Tab>>>,
    force_close: Cell<bool>,
}

/// One terminal and the program running in it.
pub struct Tab {
    win: Weak<TermWindow>,
    window: gtk::ApplicationWindow,
    settings: Rc<RefCell<Settings>>,
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
    out_src: RefCell<Option<glib::SourceId>>,
    copy_src: RefCell<Option<glib::SourceId>>,
    /// A pending refresh of `auto_title`, scheduled by activity in the tab
    /// (see `schedule_title_refresh`). None while the tab is idle.
    title_src: RefCell<Option<glib::SourceId>>,
    /// The program has printed something (for the startup time in the
    /// debug log).
    first_output: Cell<bool>,
    /// The last MAX_EVENTS events.
    events: RefCell<VecDeque<String>>,
    /// The latest event and how often it has repeated since.
    last_event: RefCell<(String, u32)>,
    /// The open Event Log window's text, kept up to date as events arrive.
    event_view: RefCell<Option<glib::WeakRef<gtk::TextBuffer>>>,
    last_size: Cell<(u16, u16)>,
    generation: Cell<u32>,
    bells: RefCell<VecDeque<Instant>>,
    bell_mute_until: Cell<Option<Instant>>,
    pad_class: RefCell<Option<String>>,
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
}

thread_local! {
    static PAD_CSS: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
    static CHROME_CSS_LOADED: Cell<bool> = const { Cell::new(false) };
    static NEXT_WINDOW: Cell<u32> = const { Cell::new(0) };
    /// The URL regex, compiled once and shared by every tab.
    static URL_REGEX: OnceCell<Result<vte::Regex, String>> = const { OnceCell::new() };
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

/// CSS class of the widgets between the window and a terminal, which are
/// cleared when the background opacity is below 100.
const CLEAR_CLASS: &str = "moriartty-clear";
const MAX_OUTQ: usize = 16 << 20;
/// How long after activity in a tab its "program: directory" name is read
/// again. Long enough for a command started with Enter to have replaced
/// the shell in the foreground.
const TITLE_REFRESH: Duration = Duration::from_millis(400);
const MAX_EVENTS: usize = 1000;
/// Copy All asks first above this many lines of scrollback.
const COPY_ALL_ASK_LINES: u64 = 1000;

/// The title bar and tabs, as one slim row (about 22px). GTK's defaults are
/// sized for touch screens, so every part that sets the bar's height gets
/// an explicit size: buttons, the window controls (and their icons, which
/// the theme pads), the tabs and their text.
const CHROME_CSS: &str = "
.moriartty-bar {
    min-height: 0;
    padding: 0 2px;
    margin: 0;
    border-bottom: 1px solid alpha(currentColor, 0.12);
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
    -gtk-icon-size: 14px;
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
.moriartty-tab:hover button,
.moriartty-tab.active button {
    opacity: 1;
}
.moriartty-tab .moriartty-log {
    color: #e01b24;
    -gtk-icon-size: 10px;
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

fn build_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let edit = gio::Menu::new();
    edit.append(Some("_Copy"), Some("win.copy"));
    edit.append(Some("_Paste"), Some("win.paste"));
    edit.append(Some("Copy _All to Clipboard"), Some("win.copy-all"));
    edit.append(Some("_Find…"), Some("win.find"));
    menu.append_section(None, &edit);

    let zoom = gio::Menu::new();
    zoom.append(Some("Zoom _In"), Some("win.zoom-in"));
    zoom.append(Some("Zoom _Out"), Some("win.zoom-out"));
    zoom.append(Some("_Normal Size"), Some("win.zoom-reset"));
    menu.append_section(None, &zoom);

    let session = gio::Menu::new();
    session.append(Some("New _Tab"), Some("win.new-tab"));
    session.append(Some("Ne_w Window"), Some("win.new-window"));
    session.append(Some("Clos_e Tab"), Some("win.close-tab"));
    session.append(Some("_Restart Shell"), Some("win.restart"));
    session.append(Some("_Settings…"), Some("win.change-settings"));
    session.append(Some("_Event Log"), Some("win.event-log"));
    menu.append_section(None, &session);

    let term = gio::Menu::new();
    term.append(Some("C_lear Scrollback"), Some("win.clear-scrollback"));
    term.append(Some("Rese_t Terminal"), Some("win.reset"));
    term.append(Some("_Full Screen"), Some("win.fullscreen"));
    menu.append_section(None, &term);

    let about = gio::Menu::new();
    about.append(Some("_About Moriartty"), Some("win.about"));
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

/// Alt+1 … Alt+9: the tab index to go to.
fn alt_digit(key: gdk::Key, ctrl: bool, shift: bool, alt: bool) -> Option<usize> {
    if !alt || ctrl || shift {
        return None;
    }
    let d = key.to_unicode()?.to_digit(10)?;
    (1..=9).contains(&d).then(|| d as usize - 1)
}

/// MORIARTTY_DEBUG: the text colour a menu really got, once it is on
/// screen. It should match the latest "title bar and menus" line; if it
/// doesn't, something else is styling the menu.
fn debug_menu_colours(popover: &gtk::Popover, which: &'static str) {
    if !super::debugging() {
        return;
    }
    popover.connect_map(move |p| {
        let p = p.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(200), move || {
            let Some(p) = p.upgrade() else { return };
            match p.first_child() {
                Some(c) => super::debug(format!(
                    "{which} menu shown: {} text {}",
                    c.css_name(),
                    c.color().to_str()
                )),
                None => super::debug(format!("{which} menu shown: no contents")),
            }
        });
    });
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
    let tab = tw.add_tab(command, cwd);
    // Problems found while loading the saved settings, e.g. a broken
    // Default Settings file that was moved aside.
    for n in storage::take_notices() {
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
        let add_btn = bar_button("list-add-symbolic", "win.new-tab", "New tab (Ctrl+Shift+T)");
        let filler = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        filler.set_hexpand(true);

        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_has_frame(false);
        menu_btn.set_menu_model(Some(&build_menu()));
        menu_btn.set_tooltip_text(Some("Menu (also Ctrl+Right-click)"));
        if let Some(p) = menu_btn.popover() {
            debug_menu_colours(&p, "title bar");
        }
        let settings_btn = bar_button(
            "preferences-system-symbolic",
            "win.change-settings",
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
            stack,
            strip,
            strip_scroll,
            title_buttons,
            css,
            css_class,
            settings_win: RefCell::new(None),
            search_bar,
            search_entry,
            search_case,
            settings: Rc::new(RefCell::new(settings)),
            tabs: RefCell::new(Vec::new()),
            force_close: Cell::new(false),
        });

        tw.apply_window_settings();
        tw.connect_signals();
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
        let mut css = String::new();
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
        self.tabs.borrow_mut().push(tab.clone());
        self.strip.append(&tab.header);
        self.stack.add_child(&tab.page);
        self.select_tab(&tab);
        glib::spawn_future_local(tab.clone().start());
        tab
    }

    /// Show a tab: its page, its header highlighted, its title on the
    /// window; then (once laid out) resize its pty and give it the focus.
    /// Pages that are not shown are not laid out, so the size is checked here.
    fn select_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
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
                    win.remove_tab(&tab);
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
        tab.popover.unparent();
        if self.tab_count() == 0 {
            self.force_close.set(true);
            self.window.close();
        } else if let (true, Some(next)) = (was_current, next) {
            self.select_tab(&next);
        }
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

    /// Dragging a tab's header: it moves to where the pointer is (`x`, in
    /// the strip's coordinates), after every other tab whose middle is to
    /// the left of the pointer.
    fn drag_tab_to(&self, tab: &Tab, x: f64) {
        let to = self
            .tabs_snapshot()
            .iter()
            .filter(|t| !std::ptr::eq(Rc::as_ptr(t), tab))
            .filter(|t| {
                t.header
                    .compute_bounds(&self.strip)
                    .is_some_and(|b| f64::from(b.x() + b.width() / 2.0) < x)
            })
            .count();
        self.move_tab_to(tab, to);
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

    /// Tab and window shortcuts. Handled for the whole window, so they work
    /// wherever the keyboard focus is (terminal, find bar, ...). The
    /// switching keys are only taken with two or more tabs, so in a
    /// single-tab window Ctrl+PgUp, Ctrl+Tab and Alt+digit still reach
    /// programs like vim and readline.
    fn tab_shortcut(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let many_tabs = self.tab_count() > 1;
        let k = key.to_lower();
        if ctrl && shift && !alt && k == gdk::Key::t {
            self.new_tab();
        } else if ctrl && shift && !alt && k == gdk::Key::w {
            if let Some(tab) = self.current_tab() {
                self.close_tab(&tab);
            }
        } else if ctrl && shift && !alt && k == gdk::Key::n {
            self.new_window();
        } else if ctrl
            && !alt
            && many_tabs
            && (key == gdk::Key::Page_Up || key == gdk::Key::Page_Down)
        {
            let forward = key == gdk::Key::Page_Down;
            if shift {
                if let Some(tab) = self.current_tab() {
                    self.move_tab(&tab, forward);
                }
            } else {
                self.cycle_tab(forward);
            }
        } else if ctrl
            && !alt
            && many_tabs
            && (key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab)
        {
            self.cycle_tab(!shift);
        } else if let Some(tab) = alt_digit(key, ctrl, shift, alt)
            .filter(|_| many_tabs)
            .and_then(|n| self.tabs_snapshot().get(n).cloned())
        {
            self.select_tab(&tab);
        } else {
            return false;
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
        keys.connect_key_pressed(move |_, key, _, state| match w.upgrade() {
            Some(tw) if tw.tab_shortcut(key, state) => glib::Propagation::Stop,
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

    fn connect_search(self: &Rc<Self>, prev_btn: &gtk::Button, next_btn: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_update();
            }
        });
        let w = Rc::downgrade(self);
        self.search_case.connect_toggled(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_update();
            }
        });
        // Enter and the up button go to older output (what you usually want
        // in a terminal); Ctrl+G / the down button go back towards newer.
        let w = Rc::downgrade(self);
        self.search_entry.connect_activate(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_find(true);
            }
        });
        let w = Rc::downgrade(self);
        self.search_entry.connect_previous_match(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_find(true);
            }
        });
        let w = Rc::downgrade(self);
        self.search_entry.connect_next_match(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_find(false);
            }
        });
        let w = Rc::downgrade(self);
        prev_btn.connect_clicked(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_find(true);
            }
        });
        let w = Rc::downgrade(self);
        next_btn.connect_clicked(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.search_find(false);
            }
        });
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
                tab.term.search_set_regex(None, 0);
                tab.term.unselect_all();
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
        self.window.add_action(&a);
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
            .window
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
            let n = t.settings.borrow().scrollback;
            t.term.set_scrollback_lines(0);
            t.term.set_scrollback_lines(i64::from(n));
        });
        self.add_tab_action("reset", |t| t.term.reset(true, false));
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
        self.add_action("about", |tw| {
            gtk::AboutDialog::builder()
                .transient_for(&tw.window)
                .modal(true)
                .program_name("Moriartty")
                .logo_icon_name(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("A fast, private-by-default terminal inspired by PuTTY's pterm")
                .license_type(gtk::License::MitX11)
                .build()
                .present();
        });
        self.add_action("change-settings", |tw| {
            // One settings window per terminal window. One that is no
            // longer shown has gone (destroyed, or on its way): never
            // present that again, open a new one instead.
            let stored = tw.settings_win.borrow().as_ref().and_then(|w| w.upgrade());
            match stored {
                Some(win) if win.is_visible() => {
                    super::debug("settings: already open, presenting it");
                    win.present();
                    return;
                }
                Some(win) => super::debug(format!(
                    "settings: stored window is no longer shown (realized: {}), opening a new one",
                    win.is_realized()
                )),
                None => super::debug("settings: opening"),
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
            super::debug(format!("settings: opened (visible: {})", win.is_visible()));
            // Forget it once destroyed: a destroyed window can linger in
            // memory, and presenting it again leaves a dead window that
            // no button closes.
            let w = Rc::downgrade(tw);
            win.connect_destroy(move |_| {
                super::debug("settings: forgotten");
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

    fn reconfigure(&self, new: Settings) {
        *self.settings.borrow_mut() = new;
        self.apply_window_settings();
        for tab in self.tabs_snapshot() {
            tab.apply_appearance();
            tab.check_size_after_layout();
            tab.update_title();
            tab.log_event("Settings changed (shell and logging changes apply on restart)");
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

        let scrollbar =
            gtk::Scrollbar::new(gtk::Orientation::Vertical, term.vadjustment().as_ref());

        // Resize hook without polling: the terminal is overlaid on an
        // invisible DrawingArea that always gets exactly the same space, and
        // DrawingArea emits "resize" whenever that space changes (window
        // resize, maximise, find bar shown, ...). The terminal itself is laid
        // out right after, so the check runs once GTK has finished the layout
        // pass. Font and zoom changes call check_size() directly, and so does
        // switching to the tab.
        let on_allocate: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::default();
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

        let popover = gtk::PopoverMenu::from_model(Some(&build_menu()));
        popover.set_parent(&term);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        debug_menu_colours(popover.upcast_ref(), "right-click");

        let tab = Rc::new(Tab {
            win: Rc::downgrade(win),
            window: win.window.clone(),
            settings: win.settings.clone(),
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
            out_src: RefCell::new(None),
            copy_src: RefCell::new(None),
            title_src: RefCell::new(None),
            first_output: Cell::new(false),
            events: RefCell::new(VecDeque::new()),
            last_event: RefCell::new((String::new(), 0)),
            event_view: RefCell::new(None),
            last_size: Cell::new((0, 0)),
            generation: Cell::new(0),
            bells: RefCell::new(VecDeque::new()),
            bell_mute_until: Cell::new(None),
            pad_class: RefCell::new(None),
            pointer: Cell::new((0.0, 0.0)),
            closed: Cell::new(false),
            ended: Cell::new(false),
            attention: Cell::new(false),
            remote_title: RefCell::new(None),
            auto_title: RefCell::new(None),
        });
        tab.apply_appearance();
        tab.update_title();
        tab.connect_signals();
        tab.connect_title_signal();

        let w = Rc::downgrade(&tab);
        close_btn.connect_clicked(move |_| {
            let Some(tab) = w.upgrade() else { return };
            if let Some(win) = tab.win.upgrade() {
                win.close_tab(&tab);
            }
        });
        // Presses on the rest of the header: the left button selects (and
        // dragging moves the tab), the middle one closes. Claimed, so they
        // don't drag the window. Presses on the close button are left
        // alone: claiming them here would cancel the button's own click,
        // and the × would do nothing.
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
            let Some(win) = tab.win.upgrade() else { return };
            if !oc(&tab.header, x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                win.close_tab(&tab);
            }
        });
        tab.header.add_controller(click);

        // Where the drag started, in the strip's coordinates; None when the
        // press was on the close button.
        let drag_origin: Rc<Cell<Option<f64>>> = Rc::default();
        let drag = gtk::GestureDrag::new();
        let (w, oc, origin) = (Rc::downgrade(&tab), on_close, drag_origin.clone());
        drag.connect_drag_begin(move |g, x, y| {
            origin.set(None);
            let Some(tab) = w.upgrade() else { return };
            let Some(win) = tab.win.upgrade() else { return };
            if oc(&tab.header, x, y) {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            win.select_tab(&tab);
            let start = graphene::Point::new(x as f32, y as f32);
            origin.set(
                tab.header
                    .compute_point(&win.strip, &start)
                    .map(|p| f64::from(p.x())),
            );
        });
        let (w, origin) = (Rc::downgrade(&tab), drag_origin);
        drag.connect_drag_update(move |_, dx, _| {
            // A small wobble while clicking is not a drag.
            let Some(x0) = origin.get().filter(|_| dx.abs() > 8.0) else {
                return;
            };
            let Some(tab) = w.upgrade() else { return };
            if let Some(win) = tab.win.upgrade() {
                win.drag_tab_to(&tab, x0 + dx);
            }
        });
        tab.header.add_controller(drag);

        let w = Rc::downgrade(&tab);
        *on_allocate.borrow_mut() = Some(Box::new(move || {
            if let Some(tab) = w.upgrade() {
                tab.check_size();
            }
        }));
        tab
    }

    // ----------------------------------------------------------------- setup

    fn apply_appearance(&self) {
        // A copy, so no borrow is held while VTE runs (it can call back
        // into this tab, see the rule at the top of this file).
        let s = self.settings.borrow().clone();
        let t = &self.term;
        t.set_font(Some(&pango::FontDescription::from_string(&s.font)));
        let c = s.active_colours(style::dark());
        let palette: Vec<gdk::RGBA> = c.ansi.iter().map(|x| rgba(*x)).collect();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        let bg = rgba(c.background);
        let alpha = s.background_opacity.clamp(30, 100) as f32 / 100.0;
        let bg = gdk::RGBA::new(bg.red(), bg.green(), bg.blue(), alpha);
        t.set_colors(Some(&rgba(c.foreground)), Some(&bg), &palette_refs);
        if super::debugging() {
            // Background opacity: what VTE was given, what it will draw,
            // and whether the display can show transparency at all.
            let display = WidgetExt::display(t);
            super::debug(format!(
                "opacity {}%: VTE background set alpha {:.2}, draws {}; display composited {}, rgba {}",
                s.background_opacity,
                bg.alpha(),
                t.color_background_for_draw().to_str(),
                display.is_composited(),
                display.is_rgba()
            ));
        }
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
        self.set_padding(s.window_border);
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
        if self.settings.borrow().allow_hyperlinks {
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
                &tab.window,
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
        let ctx = WidgetExt::display(&self.window).app_launch_context();
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
            super::message(&self.window, "Could not open the link", &e);
        }
    }

    fn set_padding(&self, px: u16) {
        let class = format!("moriartty-pad-{px}");
        PAD_CSS.with(|set| {
            if set.borrow_mut().insert(px) {
                let p = gtk::CssProvider::new();
                p.load_from_string(&format!(".{class} {{ padding: {px}px; }}"));
                gtk::style_context_add_provider_for_display(
                    &self.term.display(),
                    &p,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        });
        if let Some(old) = self.pad_class.borrow_mut().replace(class.clone()) {
            self.term.remove_css_class(&old);
        }
        self.term.add_css_class(&class);
    }

    /// The tab's name: the title the program set (unless that is turned
    /// off), else the -title / Window title setting, else "program:
    /// directory" like kitty.
    fn title(&self) -> String {
        let s = self.settings.borrow();
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
        if let Some(win) = self.win.upgrade() {
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
            if !tab.settings.borrow().copy_to_clipboard {
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
            if !g.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return;
            }
            let link = tab.link_at(x, y);
            // MORIARTTY_DEBUG: what a Ctrl+click found, to tell "no link
            // seen" apart from "link seen but could not be opened".
            if super::debugging() {
                let (m, t) = tab.term.check_match_at(x, y);
                super::debug(format!(
                    "ctrl+click at ({x:.0}, {y:.0}): match {m:?} tag {t} (url tag {:?}) -> {link:?}",
                    tab.url_tag.get()
                ));
            }
            if let Some((url, hyperlink)) = link {
                g.set_state(gtk::EventSequenceState::Claimed);
                tab.open_url(url, hyperlink);
            }
        });
        self.term.add_controller(click);

        // Mouse buttons. A capture-phase legacy controller sees presses
        // before VTE does, so right/middle clicks can be taken over.
        // Middle-click is intercepted too, so PRIMARY pastes are filtered
        // like any other paste. Shift+click passes through to VTE/the app.
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
            if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                return glib::Propagation::Proceed;
            }
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let (action, to_clipboard) = {
                let s = tab.settings.borrow();
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

        // Terminal shortcuts (PuTTY: Shift+Ins / Ctrl+Ins; Linux:
        // Ctrl+Shift+C/V/F/P; zoom: Ctrl+= / Ctrl+- / Ctrl+0). Tab and window
        // shortcuts are handled by the window (TermWindow::tab_shortcut).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = weak.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(tab) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Some(win) = tab.win.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);
            let k = key.to_lower();
            if ctrl && shift && k == gdk::Key::c {
                tab.copy();
            } else if ctrl && shift && k == gdk::Key::v {
                tab.paste(false);
            } else if ctrl && shift && k == gdk::Key::p {
                let _ = WidgetExt::activate_action(&tab.window, "win.change-settings", None);
            } else if ctrl && shift && k == gdk::Key::f {
                win.show_search();
            } else if ctrl
                && !alt
                && (key == gdk::Key::plus || key == gdk::Key::equal || key == gdk::Key::KP_Add)
            {
                win.zoom(1);
            } else if ctrl
                && !alt
                && !shift
                && (key == gdk::Key::minus || key == gdk::Key::KP_Subtract)
            {
                win.zoom(-1);
            } else if ctrl && !alt && !shift && (key == gdk::Key::_0 || key == gdk::Key::KP_0) {
                win.zoom(0);
            } else if shift && !ctrl && key == gdk::Key::Insert {
                tab.paste(false);
            } else if ctrl && !shift && key == gdk::Key::Insert {
                tab.copy();
            } else if alt
                && (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
                && tab.settings.borrow().fullscreen_on_alt_enter
            {
                win.toggle_fullscreen();
            } else {
                return glib::Propagation::Proceed;
            }
            glib::Propagation::Stop
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
                        &self.window,
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
        let notes = self.settings.borrow_mut().repair();
        for n in notes {
            self.log_event(&n);
            self.feed_status(&n);
        }
        let s = self.settings.borrow().clone();
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
        let spawned = pty::spawn(
            &s,
            self.command.as_deref(),
            self.start_dir.as_deref(),
            cols,
            rows,
            crate::sys::vte_version(),
        );
        match spawned {
            Ok((pty, desc)) => {
                self.log_event(desc);
                let fd = pty.raw_fd();
                *self.session.borrow_mut() = Some(Session { pty, log });
                self.show_logging(log_path.as_deref());
                let w = Rc::downgrade(&self);
                let id = crate::sys::unix_fd_add_local(
                    fd,
                    glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
                    move |_, _| match w.upgrade() {
                        Some(tab) => tab.on_readable(),
                        None => glib::ControlFlow::Break,
                    },
                );
                if id.is_none() {
                    self.log_event("Cannot watch the terminal for output");
                }
                *self.io_src.borrow_mut() = id;
                // The pty was created at this size; later changes arrive
                // through the layout hook (check_size).
                self.last_size.set((cols, rows));
                if self.win.upgrade().is_some_and(|w| w.is_current(&self)) {
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
        if size == self.last_size.get() {
            return;
        }
        self.last_size.set(size);
        if let Some(sess) = self.session.borrow_mut().as_mut() {
            sess.pty.resize(size.0, size.1);
        }
    }

    fn on_readable(self: &Rc<Self>) -> glib::ControlFlow {
        let mut data = Vec::new();
        let mut log_error = None;
        let status = {
            let mut g = self.session.borrow_mut();
            let Some(sess) = g.as_mut() else {
                self.io_src.borrow_mut().take();
                return glib::ControlFlow::Break;
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
            if !self.first_output.replace(true) {
                super::debug(format!(
                    "first output from the program, {} ms after start",
                    super::since_start_ms()
                ));
            }
            self.term.feed(&data);
            self.schedule_title_refresh();
        }
        match status {
            Ok(ReadStatus::More) => glib::ControlFlow::Continue,
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
        // Enter may start a program: the tab's name may be about to change.
        if bytes.contains(&b'\r') {
            self.schedule_title_refresh();
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
        let close = match self.settings.borrow().close_on_exit {
            CloseOnExit::Always => true,
            CloseOnExit::Never => false,
            CloseOnExit::Clean => clean,
        };
        if close {
            if let Some(win) = self.win.upgrade() {
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
        for src in [&self.io_src, &self.out_src, &self.copy_src, &self.title_src] {
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
                &tab.window,
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
            let (allow_controls, confirm) = {
                let s = tab.settings.borrow();
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
                    format!("Paste {lines} lines into the terminal?")
                } else {
                    "Paste text containing hidden characters?".to_string()
                };
                // Cancel is the default: an Enter pressed out of habit
                // right after pasting must not confirm it.
                let r = ask(
                    &tab.window,
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

    fn bell(&self) {
        let (style, overload, n, t, mute) = {
            let s = self.settings.borrow();
            (
                s.bell,
                s.bell_overload,
                s.bell_overload_n,
                s.bell_overload_t,
                s.bell_overload_s,
            )
        };
        // Mark background tabs, whatever the bell style.
        if !self.win.upgrade().is_some_and(|w| w.is_current(self)) && !self.attention.get() {
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
        let (x, y) = self.pointer.get();
        self.popover
            .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.popover.popup();
    }

    fn show_event_log(&self) {
        let win = gtk::Window::builder()
            .title(format!("Moriartty Event Log — {}", self.title()))
            .transient_for(&self.window)
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
    fn alt_digits() {
        assert_eq!(alt_digit(gdk::Key::_1, false, false, true), Some(0));
        assert_eq!(alt_digit(gdk::Key::_9, false, false, true), Some(8));
        assert_eq!(alt_digit(gdk::Key::_0, false, false, true), None);
        assert_eq!(alt_digit(gdk::Key::_1, false, false, false), None);
        assert_eq!(alt_digit(gdk::Key::_1, true, false, true), None);
        assert_eq!(alt_digit(gdk::Key::a, false, false, true), None);
    }
}
