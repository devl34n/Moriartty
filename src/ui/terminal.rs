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

use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use vte::prelude::*;

use super::config_dialog::{self, Mode};
use super::form::rgba;
use super::{ask, strip_controls};
use crate::config::*;
use crate::logging::{self, SessionLog};
use crate::matching::{self, PCRE2_CASELESS, PCRE2_MULTILINE, PCRE2_UTF};
use crate::paste::{paste_preview, sanitize_paste};
use crate::pty::{self, ExitState, Pty, ReadStatus};

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
    title_buttons: gtk::Box,
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
    /// Refreshes `auto_title` while the program runs.
    title_src: RefCell<Option<glib::SourceId>>,
    events: RefCell<Vec<String>>,
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
}

const MAX_OUTQ: usize = 16 << 20;

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
}
.moriartty-tab:hover {
    background-color: alpha(currentColor, 0.08);
}
.moriartty-tab.active {
    background-color: alpha(currentColor, 0.16);
}
.moriartty-tab button {
    padding: 1px;
    margin: 0;
}
";

fn load_chrome_css() {
    if CHROME_CSS_LOADED.with(|c| c.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let p = gtk::CssProvider::new();
    p.load_from_string(CHROME_CSS);
    gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
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
        Some(host) => format!(
            "Ctrl+click to open {url}\n⚠ International address: the real host is {host}"
        ),
        None => format!("Ctrl+click to open {url}"),
    }
}

/// Alt+1 … Alt+9: the tab index to go to.
fn alt_digit(key: gdk::Key, ctrl: bool, shift: bool, alt: bool) -> Option<usize> {
    if !alt || ctrl || shift {
        return None;
    }
    let d = key.to_unicode()?.to_digit(10)?;
    (1..=9).contains(&d).then(|| d as usize - 1)
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

pub fn open(app: &gtk::Application, settings: Settings, command: Option<Vec<String>>, cwd: Option<PathBuf>) {
    let tw = TermWindow::new(app, settings);
    tw.add_tab(command, cwd);
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
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let strip_scroll = gtk::ScrolledWindow::new();
        strip_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        strip_scroll.set_propagate_natural_width(true);
        strip_scroll.set_child(Some(&strip));
        let add_btn = bar_button("list-add-symbolic", "win.new-tab", "New tab (Ctrl+Shift+T)");
        let filler = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        filler.set_hexpand(true);

        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_has_frame(false);
        menu_btn.set_menu_model(Some(&build_menu()));
        menu_btn.set_tooltip_text(Some("Menu (also Ctrl+Right-click)"));
        let settings_btn = bar_button("preferences-system-symbolic", "win.change-settings", "Settings (Ctrl+Shift+P)");
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

        let tw = Rc::new(TermWindow {
            app: app.clone(),
            window,
            stack,
            strip,
            title_buttons,
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
        tw
    }

    fn apply_window_settings(&self) {
        let buttons = self.settings.borrow().title_buttons;
        self.title_buttons.set_visible(buttons);
    }

    // ------------------------------------------------------------------ tabs

    fn add_tab(self: &Rc<Self>, command: Option<Vec<String>>, start_dir: Option<PathBuf>) -> Rc<Tab> {
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
            if Rc::ptr_eq(&t, tab) {
                t.header.add_css_class("active");
            } else {
                t.header.remove_css_class("active");
            }
        }
        tab.attention.set(false);
        tab.update_label();
        self.show_tab_chrome(tab);
        let (w, t) = (Rc::downgrade(self), Rc::downgrade(tab));
        glib::idle_add_local_once(move || {
            let (Some(win), Some(tab)) = (w.upgrade(), t.upgrade()) else { return };
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
            i.and_then(|i| tabs.get(i).or_else(|| i.checked_sub(1).and_then(|j| tabs.get(j))).cloned())
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
        let next = if forward { (cur + 1) % n } else { (cur + n - 1) % n };
        self.select_tab(&tabs[next]);
    }

    /// Ctrl+Shift+PgUp / Ctrl+Shift+PgDn.
    fn move_tab(&self, tab: &Tab, forward: bool) {
        let order = {
            let mut tabs = self.tabs.borrow_mut();
            let Some(i) = tabs.iter().position(|t| std::ptr::eq(Rc::as_ptr(t), tab)) else { return };
            let j = if forward { i + 1 } else { i.wrapping_sub(1) };
            if j >= tabs.len() {
                return;
            }
            tabs.swap(i, j);
            tabs.clone()
        };
        let mut prev: Option<gtk::Widget> = None;
        for t in &order {
            self.strip.reorder_child_after(&t.header, prev.as_ref());
            prev = Some(t.header.clone().upcast());
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
        } else if ctrl && !alt && many_tabs && (key == gdk::Key::Page_Up || key == gdk::Key::Page_Down) {
            let forward = key == gdk::Key::Page_Down;
            if shift {
                if let Some(tab) = self.current_tab() {
                    self.move_tab(&tab, forward);
                }
            } else {
                self.cycle_tab(forward);
            }
        } else if ctrl && !alt && many_tabs && (key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab) {
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
                    "A program is still running in one of its tabs and will be terminated.".to_string()
                } else {
                    format!("Programs are still running in {busy} of its tabs and will be terminated.")
                };
                let tw2 = tw.clone();
                let win = win.clone();
                glib::spawn_future_local(async move {
                    let r = ask(&win, "Close this window?", &detail, &["Cancel", "Close"], 0, 1).await;
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
        self.window.connect_destroy(move |_| {
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
        self.search_bar.connect_search_mode_enabled_notify(move |bar| {
            if bar.is_search_mode() {
                return;
            }
            let Some(tab) = w.upgrade().and_then(|tw| tw.current_tab()) else { return };
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
        let Some(tab) = self.current_tab() else { return };
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
        let Some(tab) = self.current_tab() else { return };
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
        self.add_tab_action("copy-all", |t| {
            t.term.select_all();
            t.term.copy_clipboard_format(vte::Format::Text);
            t.term.unselect_all();
        });
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
                .comments("A PuTTY-style terminal emulator")
                .license_type(gtk::License::MitX11)
                .build()
                .present();
        });
        self.add_action("change-settings", |tw| {
            let w = Rc::downgrade(tw);
            let s = tw.settings.borrow().clone();
            config_dialog::show(
                &tw.app,
                s,
                Mode::Reconfigure(Rc::new(move |new| {
                    if let Some(tw) = w.upgrade() {
                        tw.reconfigure(new);
                    }
                })),
            );
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
    fn new(win: &Rc<TermWindow>, command: Option<Vec<String>>, start_dir: Option<PathBuf>) -> Rc<Self> {
        let (columns, rows) = {
            let s = win.settings.borrow();
            (s.columns, s.rows)
        };
        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_size(i64::from(columns), i64::from(rows));

        let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, term.vadjustment().as_ref());

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
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.add_css_class("moriartty-tab");
        header.append(&label);
        header.append(&close_btn);

        let popover = gtk::PopoverMenu::from_model(Some(&build_menu()));
        popover.set_parent(&term);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);

        let tab = Rc::new(Tab {
            win: Rc::downgrade(win),
            window: win.window.clone(),
            settings: win.settings.clone(),
            page,
            header,
            label,
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
            events: RefCell::new(Vec::new()),
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
        // Clicks on the rest of the header: select, middle-click closes.
        // Claimed, so they don't drag the window. Presses on the close
        // button are left alone: claiming them here would cancel the
        // button's own click, and the × would do nothing.
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let w = Rc::downgrade(&tab);
        let close_ref = close_btn.downgrade();
        click.connect_pressed(move |g, _, x, y| {
            let Some(tab) = w.upgrade() else { return };
            let Some(win) = tab.win.upgrade() else { return };
            let on_close = match (tab.header.pick(x, y, gtk::PickFlags::DEFAULT), close_ref.upgrade()) {
                (Some(hit), Some(close)) => hit == *close.upcast_ref::<gtk::Widget>() || hit.is_ancestor(&close),
                _ => false,
            };
            if on_close {
                return;
            }
            match g.current_button() {
                1 => {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    win.select_tab(&tab);
                }
                2 => {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    win.close_tab(&tab);
                }
                _ => {}
            }
        });
        tab.header.add_controller(click);

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
        let s = self.settings.borrow();
        let t = &self.term;
        t.set_font(Some(&pango::FontDescription::from_string(&s.font)));
        let c = &s.colours;
        let palette: Vec<gdk::RGBA> = c.ansi.iter().map(|x| rgba(*x)).collect();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        t.set_colors(Some(&rgba(c.foreground)), Some(&rgba(c.background)), &palette_refs);
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
        let (border, urls) = (s.window_border, s.clickable_urls);
        drop(s);
        self.set_padding(border);
        self.set_url_matching(urls);
    }

    /// Highlight http(s) URLs (pointer cursor on hover) so Ctrl+click can
    /// open them.
    fn set_url_matching(&self, enabled: bool) {
        match (enabled, self.url_tag.get()) {
            (true, None) => {
                match vte::Regex::for_match(
                    matching::URL_REGEX,
                    PCRE2_MULTILINE | PCRE2_UTF | PCRE2_CASELESS,
                ) {
                    Ok(re) => {
                        let tag = self.term.match_add_regex(&re, 0);
                        self.term.match_set_cursor_name(tag, "pointer");
                        self.url_tag.set(Some(tag));
                    }
                    Err(e) => self.log_event(format!("URL matching unavailable: {e}")),
                }
            }
            (false, Some(tag)) => {
                self.term.match_remove(tag);
                self.url_tag.set(None);
            }
            _ => {}
        }
    }

    /// The safe http(s) link under widget coordinates (x, y), if any:
    /// a matched URL, or an OSC 8 hyperlink when those are allowed.
    fn link_at(&self, x: f64, y: f64) -> Option<String> {
        if let Some(tag) = self.url_tag.get() {
            let (m, t) = self.term.check_match_at(x, y);
            if t == tag {
                if let Some(url) = m.and_then(|m| matching::safe_url(&m)) {
                    return Some(url);
                }
            }
        }
        if self.settings.borrow().allow_hyperlinks {
            if let Some(url) = self.term.check_hyperlink_at(x, y).and_then(|h| matching::safe_url(&h)) {
                return Some(url);
            }
        }
        None
    }

    fn update_link_tooltip(&self, x: f64, y: f64) {
        let link = self.link_at(x, y);
        if *self.hover_url.borrow() == link {
            return;
        }
        self.term.set_tooltip_text(link.as_deref().map(link_tooltip).as_deref());
        *self.hover_url.borrow_mut() = link;
    }

    fn open_url(&self, url: &str) {
        match matching::idn_ascii_host(url) {
            Some(host) => self.log_event(format!("Opening {url} (real host {host})")),
            None => self.log_event(format!("Opening {url}")),
        }
        gtk::UriLauncher::new(url).launch(
            Some(&self.window),
            None::<&gio::Cancellable>,
            |_| {},
        );
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
        let remote = self.remote_title.borrow().clone().filter(|_| !s.no_remote_wintitle);
        let base = remote
            .or_else(|| (!s.wintitle.is_empty()).then(|| s.wintitle.clone()))
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
    }

    fn update_title(&self) {
        self.update_label();
        if let Some(win) = self.win.upgrade() {
            win.tab_changed(self);
        }
    }

    /// Re-read "program: directory" (polled: nothing signals a change of
    /// foreground program or directory).
    fn refresh_auto_title(&self) {
        let t = {
            let g = self.session.borrow();
            g.as_ref().and_then(|s| s.pty.describe()).map(|t| strip_controls(&t, 200))
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

        // Copy on select (debounced, so dragging doesn't spam the clipboard).
        let w = weak.clone();
        self.term.connect_selection_changed(move |_| {
            let Some(tab) = w.upgrade() else { return };
            if !tab.settings.borrow().copy_on_select {
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
            if let Some(url) = tab.link_at(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                tab.open_url(&url);
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
            let Some(tab) = w.upgrade() else { return glib::Propagation::Proceed };
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
            let action = tab.settings.borrow().mouse_action;
            match (be.button(), ctrl) {
                (3, true) => tab.show_menu(),
                (3, false) => match action {
                    MouseAction::Compromise => tab.paste(false),
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
            let Some(tab) = w.upgrade() else { return glib::Propagation::Proceed };
            let Some(win) = tab.win.upgrade() else { return glib::Propagation::Proceed };
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
            } else if ctrl && !alt && !shift && (key == gdk::Key::minus || key == gdk::Key::KP_Subtract) {
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

    fn log_event(&self, msg: impl AsRef<str>) {
        let ts = glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%Y-%m-%d %H:%M:%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let line = format!("{ts}\t{}", strip_controls(msg.as_ref(), 4000));
        let view = self.event_view.borrow().as_ref().and_then(|w| w.upgrade());
        if let Some(buf) = view {
            let sep = if buf.char_count() > 0 { "\n" } else { "" };
            buf.insert(&mut buf.end_iter(), &format!("{sep}{line}"));
        }
        self.events.borrow_mut().push(line);
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
        if s.log_type != LogType::None {
            match self.prepare_log(&s).await {
                Ok(Some((path, file))) => {
                    self.log_event(format!("Logging to {}", path.display()));
                    match SessionLog::new(file, s.log_type, s.log_flush, s.log_header) {
                        Ok(l) => log = Some(l),
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
                let w = Rc::downgrade(&self);
                let id = glib::unix_fd_add_local(
                    fd,
                    glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
                    move |_, _| match w.upgrade() {
                        Some(tab) => tab.on_readable(),
                        None => glib::ControlFlow::Break,
                    },
                );
                *self.io_src.borrow_mut() = Some(id);
                // The tab name follows the foreground program and directory.
                // Removed in shutdown(), which runs before the tab is dropped.
                let w = Rc::downgrade(&self);
                let id = glib::timeout_add_local(Duration::from_secs(1), move || match w.upgrade() {
                    Some(tab) => {
                        tab.refresh_auto_title();
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                });
                *self.title_src.borrow_mut() = Some(id);
                // The pty was created at this size; later changes arrive
                // through the layout hook (check_size).
                self.last_size.set((cols, rows));
                if self.win.upgrade().is_some_and(|w| w.is_current(&self)) {
                    self.term.grab_focus();
                }
                // Once the shell has had a moment to start.
                let w = Rc::downgrade(&self);
                glib::timeout_add_local_once(Duration::from_millis(200), move || {
                    if let Some(tab) = w.upgrade() {
                        tab.refresh_auto_title();
                    }
                });
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
        }
        if !data.is_empty() {
            self.term.feed(&data);
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
                let id = glib::unix_fd_add_local(sess.pty.raw_fd(), glib::IOCondition::OUT, move |_, _| {
                    if let Some(tab) = w.upgrade() {
                        tab.out_src.borrow_mut().take();
                        tab.flush();
                    }
                    glib::ControlFlow::Break
                });
                *self.out_src.borrow_mut() = Some(id);
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
            (None, ExitState::Running) => ("Terminal closed; process terminated".to_string(), false),
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
        self.feed_status(&format!("{msg}. Ctrl+Right-click → Restart Shell to start again."));
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
            if confirm && breaks > 0 {
                let lines = breaks + usize::from(!clean.ends_with('\r'));
                // Cancel is the default: an Enter pressed out of habit
                // right after pasting must not confirm it.
                let r = ask(
                    &tab.window,
                    &format!("Paste {lines} lines into the terminal?"),
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
            let alive = !tab.closed.get() && tab.generation.get() == gen && tab.session.borrow().is_some();
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
            (s.bell, s.bell_overload, s.bell_overload_n, s.bell_overload_t, s.bell_overload_s)
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
                self.bell_mute_until.set(Some(now + Duration::from_secs(u64::from(mute))));
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
                glib::timeout_add_local_once(Duration::from_millis(90), move || term.set_opacity(1.0));
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
        buf.set_text(&self.events.borrow().join("\n"));
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
