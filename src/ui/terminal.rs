//! The terminal window: a VTE widget attached to a shell on a pty.
//!
//! Data flow
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
use std::rc::Rc;
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
use crate::paste::sanitize_paste;
use crate::pty::{self, ExitState, Pty, ReadStatus};

struct Session {
    pty: Pty,
    log: Option<SessionLog>,
}

pub struct TermWindow {
    app: gtk::Application,
    window: gtk::ApplicationWindow,
    term: vte::Terminal,
    scrollbar: gtk::Scrollbar,
    popover: gtk::PopoverMenu,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_case: gtk::CheckButton,
    /// VTE match tag of the URL regex, while clickable URLs are enabled.
    url_tag: Cell<Option<i32>>,
    hover_url: RefCell<Option<String>>,
    settings: RefCell<Settings>,
    /// `-e` command; `None` runs the shell.
    command: Option<Vec<String>>,
    start_dir: Option<PathBuf>,
    session: RefCell<Option<Session>>,
    outq: RefCell<Vec<u8>>,
    io_src: RefCell<Option<glib::SourceId>>,
    out_src: RefCell<Option<glib::SourceId>>,
    copy_src: RefCell<Option<glib::SourceId>>,
    events: RefCell<Vec<String>>,
    last_size: Cell<(u16, u16)>,
    generation: Cell<u32>,
    bells: RefCell<VecDeque<Instant>>,
    bell_mute_until: Cell<Option<Instant>>,
    pad_class: RefCell<Option<String>>,
    pointer: Cell<(f64, f64)>,
    force_close: Cell<bool>,
    ended: Cell<bool>,
    remote_title: RefCell<Option<String>>,
}

thread_local! {
    static PAD_CSS: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
}

const MAX_OUTQ: usize = 16 << 20;

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
    session.append(Some("Ne_w Window"), Some("win.new-window"));
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

pub fn open(app: &gtk::Application, settings: Settings, command: Option<Vec<String>>, cwd: Option<PathBuf>) {
    let tw = TermWindow::new(app, settings, command, cwd);
    tw.window.present();
    glib::spawn_future_local(tw.clone().start());
}

impl TermWindow {
    fn new(
        app: &gtk::Application,
        settings: Settings,
        command: Option<Vec<String>>,
        start_dir: Option<PathBuf>,
    ) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .icon_name(crate::APP_ID)
            .build();

        // Title bar: settings (gear) and the full menu, so neither depends on
        // knowing about Ctrl+Right-click.
        let header = gtk::HeaderBar::new();
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&build_menu()));
        menu_btn.set_tooltip_text(Some("Menu (also Ctrl+Right-click)"));
        let settings_btn = gtk::Button::from_icon_name("preferences-system-symbolic");
        settings_btn.set_action_name(Some("win.change-settings"));
        settings_btn.set_tooltip_text(Some("Settings (Ctrl+Shift+P)"));
        header.pack_end(&menu_btn);
        header.pack_end(&settings_btn);
        window.set_titlebar(Some(&header));

        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_size(i64::from(settings.columns), i64::from(settings.rows));

        let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, term.vadjustment().as_ref());
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hbox.append(&term);
        hbox.append(&scrollbar);

        // Find bar (Ctrl+Shift+F), hidden until used.
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
        vbox.append(&hbox);
        window.set_child(Some(&vbox));

        let popover = gtk::PopoverMenu::from_model(Some(&build_menu()));
        popover.set_parent(&term);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);

        let tw = Rc::new(TermWindow {
            app: app.clone(),
            window,
            term,
            scrollbar,
            popover,
            search_bar,
            search_entry,
            search_case,
            url_tag: Cell::new(None),
            hover_url: RefCell::new(None),
            settings: RefCell::new(settings),
            command,
            start_dir,
            session: RefCell::new(None),
            outq: RefCell::new(Vec::new()),
            io_src: RefCell::new(None),
            out_src: RefCell::new(None),
            copy_src: RefCell::new(None),
            events: RefCell::new(Vec::new()),
            last_size: Cell::new((0, 0)),
            generation: Cell::new(0),
            bells: RefCell::new(VecDeque::new()),
            bell_mute_until: Cell::new(None),
            pad_class: RefCell::new(None),
            pointer: Cell::new((0.0, 0.0)),
            force_close: Cell::new(false),
            ended: Cell::new(false),
            remote_title: RefCell::new(None),
        });
        tw.apply_appearance();
        tw.update_title();
        tw.connect_signals();
        tw.connect_title_signal();
        tw.connect_search(&prev_btn, &next_btn);
        tw.install_actions();
        tw
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
        self.term
            .set_tooltip_text(link.as_ref().map(|u| format!("Ctrl+click to open {u}")).as_deref());
        *self.hover_url.borrow_mut() = link;
    }

    fn open_url(&self, url: &str) {
        self.log_event(format!("Opening {url}"));
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

    fn update_title(&self) {
        let title = {
            let s = self.settings.borrow();
            let remote = self.remote_title.borrow().clone().filter(|_| !s.no_remote_wintitle);
            let base = remote.unwrap_or_else(|| {
                if s.wintitle.is_empty() {
                    "Moriartty".to_string()
                } else {
                    s.wintitle.clone()
                }
            });
            if self.ended.get() {
                format!("{base} (inactive)")
            } else {
                base
            }
        };
        self.window.set_title(Some(&title));
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);

        // Everything the terminal wants to send: keystrokes, pastes, replies.
        let w = weak.clone();
        self.term.connect_commit(move |_, text, _| {
            if let Some(tw) = w.upgrade() {
                tw.send_user(text.as_bytes());
            }
        });

        // Copy on select (debounced, so dragging doesn't spam the clipboard).
        let w = weak.clone();
        self.term.connect_selection_changed(move |_| {
            let Some(tw) = w.upgrade() else { return };
            if !tw.settings.borrow().copy_on_select {
                return;
            }
            if let Some(id) = tw.copy_src.borrow_mut().take() {
                id.remove();
            }
            let w2 = Rc::downgrade(&tw);
            let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                if let Some(tw) = w2.upgrade() {
                    tw.copy_src.borrow_mut().take();
                    if tw.term.has_selection() {
                        tw.term.copy_clipboard_format(vte::Format::Text);
                    }
                }
            });
            *tw.copy_src.borrow_mut() = Some(id);
        });

        let w = weak.clone();
        self.term.connect_bell(move |_| {
            if let Some(tw) = w.upgrade() {
                tw.bell();
            }
        });

        // Pointer position, for placing the context menu and link tooltips.
        let motion = gtk::EventControllerMotion::new();
        let w = weak.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(tw) = w.upgrade() {
                tw.pointer.set((x, y));
                tw.update_link_tooltip(x, y);
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
            let Some(tw) = w.upgrade() else { return };
            if !g.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return;
            }
            if let Some(url) = tw.link_at(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                tw.open_url(&url);
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
            let Some(tw) = w.upgrade() else { return glib::Propagation::Proceed };
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
            let action = tw.settings.borrow().mouse_action;
            match (be.button(), ctrl) {
                (3, true) => tw.show_menu(),
                (3, false) => match action {
                    MouseAction::Compromise => tw.paste(false),
                    MouseAction::Windows => tw.show_menu(),
                    MouseAction::Xterm => return glib::Propagation::Proceed,
                },
                (2, _) => tw.paste(true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.term.add_controller(legacy);

        // Keyboard shortcuts (PuTTY: Shift+Ins / Ctrl+Ins; Linux: Ctrl+Shift+C/V/N/F/P;
        // zoom: Ctrl+= / Ctrl+- / Ctrl+0).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = weak.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(tw) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);
            let k = key.to_lower();
            if ctrl && shift && k == gdk::Key::c {
                tw.copy();
            } else if ctrl && shift && k == gdk::Key::v {
                tw.paste(false);
            } else if ctrl && shift && k == gdk::Key::n {
                tw.new_window();
            } else if ctrl && shift && k == gdk::Key::p {
                let _ = WidgetExt::activate_action(&tw.window, "win.change-settings", None);
            } else if ctrl && shift && k == gdk::Key::f {
                tw.show_search();
            } else if ctrl
                && !alt
                && (key == gdk::Key::plus || key == gdk::Key::equal || key == gdk::Key::KP_Add)
            {
                tw.zoom(1);
            } else if ctrl && !alt && !shift && (key == gdk::Key::minus || key == gdk::Key::KP_Subtract) {
                tw.zoom(-1);
            } else if ctrl && !alt && !shift && (key == gdk::Key::_0 || key == gdk::Key::KP_0) {
                tw.zoom(0);
            } else if shift && !ctrl && key == gdk::Key::Insert {
                tw.paste(false);
            } else if ctrl && !shift && key == gdk::Key::Insert {
                tw.copy();
            } else if alt
                && (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
                && tw.settings.borrow().fullscreen_on_alt_enter
            {
                tw.toggle_fullscreen();
            } else {
                return glib::Propagation::Proceed;
            }
            glib::Propagation::Stop
        });
        self.term.add_controller(keys);

        // Warn before closing while a program is running. This closure holds
        // the strong reference that keeps the TermWindow alive for the
        // window's lifetime.
        let strong = self.clone();
        self.window.connect_close_request(move |win| {
            let tw = &strong;
            let busy = tw
                .session
                .borrow()
                .as_ref()
                .is_some_and(|s| s.pty.has_foreground_job());
            if !tw.force_close.get() && busy && tw.settings.borrow().warn_on_close {
                let tw2 = tw.clone();
                let win = win.clone();
                glib::spawn_future_local(async move {
                    let r = ask(
                        &win,
                        "Close this terminal?",
                        "A program is still running in it and will be terminated.",
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
            tw.shutdown();
            glib::Propagation::Proceed
        });

        let popover = self.popover.clone();
        self.window.connect_destroy(move |_| popover.unparent());
    }

    #[allow(deprecated)]
    fn connect_title_signal(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.term.connect_window_title_changed(move |t| {
            let Some(tw) = w.upgrade() else { return };
            let title = t.window_title().map(|s| strip_controls(&s, 200));
            *tw.remote_title.borrow_mut() = title.filter(|s| !s.trim().is_empty());
            tw.update_title();
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
            if let Some(tw) = w.upgrade() {
                tw.term.search_set_regex(None, 0);
                tw.term.unselect_all();
                tw.term.grab_focus();
            }
        });
    }

    fn show_search(&self) {
        self.search_bar.set_search_mode(true);
        self.search_entry.grab_focus();
        self.search_entry.select_region(0, -1);
    }

    fn search_update(&self) {
        let text = self.search_entry.text();
        self.search_entry.remove_css_class("error");
        if text.is_empty() {
            self.term.search_set_regex(None, 0);
            return;
        }
        let mut flags = PCRE2_MULTILINE | PCRE2_UTF;
        if !self.search_case.is_active() {
            flags |= PCRE2_CASELESS;
        }
        match vte::Regex::for_search(&matching::regex_escape(&text), flags) {
            Ok(re) => {
                self.term.search_set_regex(Some(&re), 0);
                self.term.search_set_wrap_around(true);
                self.term.unselect_all();
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
        let found = if older {
            self.term.search_find_previous()
        } else {
            self.term.search_find_next()
        };
        if found {
            self.search_entry.remove_css_class("error");
        } else {
            self.search_entry.add_css_class("error");
        }
    }

    /// Per-window font zoom: 1 bigger, -1 smaller, 0 back to normal.
    fn zoom(&self, step: i32) {
        let scale = match step {
            0 => 1.0,
            s if s > 0 => self.term.font_scale() * 1.1,
            _ => self.term.font_scale() / 1.1,
        };
        self.term.set_font_scale(scale.clamp(0.3, 4.0));
    }

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
        self.add_action("copy", |tw| tw.copy());
        self.add_action("paste", |tw| tw.paste(false));
        self.add_action("copy-all", |tw| {
            tw.term.select_all();
            tw.term.copy_clipboard_format(vte::Format::Text);
            tw.term.unselect_all();
        });
        self.add_action("clear-scrollback", |tw| {
            let n = tw.settings.borrow().scrollback;
            tw.term.set_scrollback_lines(0);
            tw.term.set_scrollback_lines(i64::from(n));
        });
        self.add_action("reset", |tw| tw.term.reset(true, false));
        self.add_action("find", |tw| tw.show_search());
        self.add_action("zoom-in", |tw| tw.zoom(1));
        self.add_action("zoom-out", |tw| tw.zoom(-1));
        self.add_action("zoom-reset", |tw| tw.zoom(0));
        self.add_action("event-log", |tw| tw.show_event_log());
        self.add_action("fullscreen", |tw| tw.toggle_fullscreen());
        self.add_action("new-window", |tw| tw.new_window());
        self.add_action("restart", |tw| tw.restart());
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

    // -------------------------------------------------------------- session

    fn log_event(&self, msg: impl AsRef<str>) {
        let ts = glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%Y-%m-%d %H:%M:%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        self.events
            .borrow_mut()
            .push(format!("{ts}\t{}", strip_controls(msg.as_ref(), 4000)));
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
                            "{}\n\nOverwrite it, append to it, or disable logging for this window?",
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
                Ok(None) => self.log_event("Logging disabled for this window"),
                Err(e) => {
                    self.log_event(&e);
                    self.feed_status(&e);
                }
            }
        }
        if self.generation.get() != gen || self.force_close.get() {
            return; // closed or restarted while the log dialog was open
        }

        let (cols, rows) = self.term_size();
        match pty::spawn(&s, self.command.as_deref(), self.start_dir.as_deref(), cols, rows) {
            Ok((pty, desc)) => {
                self.log_event(desc);
                let fd = pty.raw_fd();
                *self.session.borrow_mut() = Some(Session { pty, log });
                let w = Rc::downgrade(&self);
                let id = glib::unix_fd_add_local(
                    fd,
                    glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
                    move |_, _| match w.upgrade() {
                        Some(tw) => tw.on_readable(),
                        None => glib::ControlFlow::Break,
                    },
                );
                *self.io_src.borrow_mut() = Some(id);
                self.start_resize_watch(gen);
                self.term.grab_focus();
            }
            Err(e) => {
                self.log_event(&e);
                self.feed_status(&e);
                self.ended.set(true);
                self.set_action_enabled("restart", true);
                self.update_title();
            }
        }
    }

    fn start_resize_watch(self: &Rc<Self>, gen: u32) {
        self.last_size.set((0, 0));
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(150), move || {
            let Some(tw) = w.upgrade() else { return glib::ControlFlow::Break };
            if tw.ended.get() || tw.generation.get() != gen {
                return glib::ControlFlow::Break;
            }
            tw.check_size();
            glib::ControlFlow::Continue
        });
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
                    if let Some(tw) = w.upgrade() {
                        tw.out_src.borrow_mut().take();
                        tw.flush();
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
                if let Some(tw) = w.upgrade() {
                    tw.poll_exit(err, attempt + 1);
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
            self.force_close.set(true);
            self.window.close();
            return;
        }
        self.feed_status(&format!("{msg}. Ctrl+Right-click → Restart Shell to start again."));
        self.set_action_enabled("restart", true);
        self.update_title();
    }

    /// Stop all I/O and drop the pty (which hangs up the child).
    fn shutdown(&self) {
        self.ended.set(true);
        for src in [&self.io_src, &self.out_src, &self.copy_src] {
            if let Some(id) = src.borrow_mut().take() {
                id.remove();
            }
        }
        let sess = self.session.borrow_mut().take();
        drop(sess);
        self.outq.borrow_mut().clear();
    }

    fn restart(self: &Rc<Self>) {
        if !self.ended.get() {
            return;
        }
        self.ended.set(false);
        self.remote_title.borrow_mut().take();
        self.set_action_enabled("restart", false);
        self.update_title();
        self.term.reset(true, false);
        glib::spawn_future_local(self.clone().start());
    }

    fn new_window(&self) {
        let cwd = self.session.borrow().as_ref().and_then(|s| s.pty.cwd());
        let s = self.settings.borrow().clone();
        open(&self.app, s, None, cwd);
    }

    fn reconfigure(self: &Rc<Self>, new: Settings) {
        *self.settings.borrow_mut() = new;
        self.apply_appearance();
        self.update_title();
        self.log_event("Settings changed (shell and logging changes apply on restart)");
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
        let tw = self.clone();
        glib::spawn_future_local(async move {
            let text = match clip.read_text_future().await {
                Ok(Some(t)) => t.to_string(),
                _ => return,
            };
            let (allow_controls, confirm) = {
                let s = tw.settings.borrow();
                (s.paste_controls, s.confirm_multiline_paste)
            };
            let clean = sanitize_paste(&text, allow_controls);
            if clean.is_empty() {
                return;
            }
            let breaks = clean.matches('\r').count();
            if confirm && breaks > 0 {
                let lines = breaks + usize::from(!clean.ends_with('\r'));
                let mut preview: String = clean
                    .chars()
                    .take(400)
                    .map(|c| if c == '\r' { '\n' } else { c })
                    .collect();
                if clean.chars().count() > 400 {
                    preview.push('…');
                }
                let r = ask(
                    &tw.window,
                    &format!("Paste {lines} lines into the terminal?"),
                    &preview,
                    &["Cancel", "Paste"],
                    0,
                    1,
                )
                .await;
                if r != 1 {
                    return;
                }
            }
            let alive = tw.session.borrow().is_some();
            if alive {
                // paste_text honours bracketed-paste mode.
                tw.term.paste_text(&clean);
            }
        });
    }

    // ---------------------------------------------------------------- misc

    fn bell(&self) {
        let (style, overload, n, t, mute) = {
            let s = self.settings.borrow();
            (s.bell, s.bell_overload, s.bell_overload_n, s.bell_overload_t, s.bell_overload_s)
        };
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

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    fn show_event_log(&self) {
        let win = gtk::Window::builder()
            .title("Moriartty Event Log")
            .transient_for(&self.window)
            .default_width(720)
            .default_height(380)
            .build();
        let buf = gtk::TextBuffer::new(None);
        buf.set_text(&self.events.borrow().join("\n"));
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

impl Drop for TermWindow {
    fn drop(&mut self) {
        self.shutdown();
    }
}
