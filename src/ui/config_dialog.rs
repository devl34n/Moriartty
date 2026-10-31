//! The PuTTY/pterm-style settings window: category tree on the left, option
//! pages on the right, saved sessions on the Session page.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;

use super::form::{group, note, page, Form};
use super::keys;
use crate::config::*;
use crate::{argv, storage, ui};

/// Everything the settings window checks before using or saving settings.
fn check(s: &Settings) -> Result<(), String> {
    s.validate()?;
    keys::check(&s.keys)
}

/// Shows settings in the terminal without applying them.
pub type Preview = Rc<dyn Fn(&Settings)>;

pub enum Mode {
    /// Before any terminal: Open starts one, with this command and directory.
    New {
        command: Option<Vec<String>>,
        dir: Option<PathBuf>,
    },
    /// Changing an open terminal window. `preview` shows the look of the
    /// settings being edited live; `apply` makes them stick. Closing the
    /// dialog without applying previews the original settings again.
    Reconfigure {
        apply: Rc<dyn Fn(Settings)>,
        preview: Preview,
    },
}

struct Ctx {
    window: gtk::ApplicationWindow,
    /// The session highlighted in the Session page's list: Apply (or Open)
    /// saves the settings to it. None saves nowhere.
    selected: Rc<RefCell<Option<String>>>,
    reconfig: bool,
}

type Builder = fn(&Form, &Ctx) -> gtk::Box;

const PAGES: &[(i32, &str, &str, Builder)] = &[
    (0, "session", "Session", session_page),
    (1, "logging", "Logging", logging_page),
    (0, "terminal", "Terminal", terminal_page),
    (1, "keyboard", "Keyboard", keyboard_page),
    (1, "bell", "Bell", bell_page),
    (1, "features", "Features", features_page),
    (0, "window", "Window", window_page),
    (1, "appearance", "Appearance", appearance_page),
    (1, "behaviour", "Behaviour", behaviour_page),
    (1, "selection", "Selection", selection_page),
    (1, "colours", "Colours", colours_page),
];

pub fn show(app: &gtk::Application, initial: Settings, mode: Mode) -> gtk::ApplicationWindow {
    let reconfig = matches!(mode, Mode::Reconfigure { .. });
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .icon_name(crate::APP_ID)
        .title("Moriartty Configuration")
        .default_width(780)
        .default_height(620)
        .build();
    let applied = Rc::new(std::cell::Cell::new(false));
    // Put the terminal's previewed look back, unless the settings were
    // applied. Run after this window has gone, so closing never waits on
    // restyling the terminal.
    let revert: Rc<dyn Fn()> = match &mode {
        Mode::Reconfigure { preview, .. } => {
            let original = initial.clone();
            let (preview, applied) = (preview.clone(), applied.clone());
            Rc::new(move || {
                if !applied.replace(true) {
                    let (preview, original) = (preview.clone(), original.clone());
                    gtk::glib::idle_add_local_once(move || preview(&original));
                }
            })
        }
        Mode::New { .. } => Rc::new(|| {}),
    };
    {
        let revert = revert.clone();
        window.connect_close_request(move |_| {
            revert();
            gtk::glib::Propagation::Proceed
        });
    }
    // The window's own session is highlighted to begin with, so Apply saves
    // back to where its settings came from. Not with command-line options:
    // they are for this window, and a one-off -log saved to Default Settings
    // would log every window from then on.
    let selected = Rc::new(RefCell::new((!initial.cli_options).then(|| {
        initial
            .session
            .clone()
            .unwrap_or_else(|| storage::DEFAULT_SESSION.to_string())
    })));
    let form = Form::new(initial);
    {
        let form = form.clone();
        window.connect_destroy(move |_| form.clear());
    }
    // Live preview in the terminal, when there is one (Reconfigure): every
    // change in the form, a picked colour scheme included, shows at once.
    let preview: Option<Preview> = match &mode {
        Mode::Reconfigure { preview, .. } => Some(preview.clone()),
        Mode::New { .. } => None,
    };
    {
        let (preview, s) = (preview.clone(), form.s.clone());
        form.on_change(move || {
            if let Some(preview) = &preview {
                preview(&s.borrow());
            }
        });
    }

    // Apply (Open) uses the settings and saves them to the highlighted
    // session, so they are still there in the next window.
    let open: Rc<dyn Fn()> = {
        let form = form.clone();
        let window = window.clone();
        let app = app.clone();
        let selected = selected.clone();
        let applied = applied.clone();
        Rc::new(move || {
            let mut s = form.get();
            s.sanitize();
            if let Err(e) = check(&s) {
                ui::message(&window, "Invalid settings", &e);
                return;
            }
            let target = selected.borrow().clone();
            let saved = match &target {
                Some(name) => storage::save_session(name, &s),
                None => Ok(()),
            };
            if target.is_some() {
                s.session = target;
                s.cli_options = false;
            }
            applied.set(true);
            match &mode {
                Mode::New { command, dir } => {
                    ui::terminal::open(&app, s, command.clone(), dir.clone())
                }
                Mode::Reconfigure { apply, .. } => apply(s),
            }
            // Once applied, the dialog always goes (destroyed outright, not
            // through close-request). A save error is shown on the terminal
            // window instead of holding this one open.
            window.destroy();
            if let Err(e) = saved {
                eprintln!("moriartty: could not save settings: {e}");
                let parent = app
                    .windows()
                    .into_iter()
                    .find(|w| w != window.upcast_ref::<gtk::Window>());
                if let Some(parent) = parent {
                    ui::message(&parent, "Settings applied, but could not be saved", &e);
                }
            }
        })
    };

    let ctx = Ctx {
        window: window.clone(),
        selected,
        reconfig,
    };

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    let nav = gtk::ListBox::new();
    nav.set_selection_mode(gtk::SelectionMode::Single);

    for &(level, id, title, build) in PAGES {
        let content = build(&form, &ctx);
        let sw = gtk::ScrolledWindow::new();
        sw.set_hscrollbar_policy(gtk::PolicyType::Never);
        sw.set_child(Some(&content));
        stack.add_named(&sw, Some(id));

        let label = gtk::Label::new(Some(title));
        label.set_xalign(0.0);
        label.set_margin_start(8 + 16 * level);
        label.set_margin_end(12);
        label.set_margin_top(3);
        label.set_margin_bottom(3);
        if level == 0 {
            label.add_css_class("heading");
        }
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        row.set_widget_name(id);
        nav.append(&row);
    }
    {
        let stack = stack.clone();
        nav.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                stack.set_visible_child_name(&row.widget_name());
            }
        });
    }
    nav.select_row(nav.row_at_index(0).as_ref());

    let nav_sw = gtk::ScrolledWindow::new();
    nav_sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    nav_sw.set_min_content_width(170);
    nav_sw.set_child(Some(&nav));
    let category = gtk::Frame::new(Some("Category"));
    category.set_child(Some(&nav_sw));

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    body.append(&category);
    body.append(&stack);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let open_btn = gtk::Button::with_label(if reconfig { "Apply" } else { "Open" });
    open_btn.add_css_class("suggested-action");
    {
        let (window, revert) = (window.clone(), revert.clone());
        cancel.connect_clicked(move |_| {
            revert();
            window.destroy();
        });
    }
    {
        let open = open.clone();
        open_btn.connect_clicked(move |_| open());
    }
    buttons.append(&cancel);
    buttons.append(&open_btn);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    root.set_margin_start(8);
    root.set_margin_end(8);
    root.append(&body);
    root.append(&buttons);
    window.set_child(Some(&root));
    window.set_default_widget(Some(&open_btn));

    form.refresh();
    window.present();
    window
}

// ------------------------------------------------------------------ Session

fn fill_sessions(list: &gtk::ListBox) {
    list.remove_all();
    for name in storage::list_sessions() {
        let l = gtk::Label::new(Some(&name));
        l.set_xalign(0.0);
        l.set_margin_start(6);
        l.set_margin_top(2);
        l.set_margin_bottom(2);
        list.append(&l);
    }
}

fn row_name(row: &gtk::ListBoxRow) -> Option<String> {
    row.child()
        .and_downcast::<gtk::Label>()
        .map(|l| l.text().to_string())
}

/// Highlight the session called `name`; false if it isn't in the list.
fn select_named(list: &gtk::ListBox, name: &str) -> bool {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        if row_name(&row).as_deref() == Some(name) {
            list.select_row(Some(&row));
            return true;
        }
        i += 1;
    }
    false
}

fn load_into(form: &Form, window: &gtk::ApplicationWindow, name: &str) -> bool {
    match storage::load_session(name) {
        Ok(s) => {
            form.set_all(s);
            true
        }
        Err(e) => {
            ui::message(window, "Could not load session", &e);
            false
        }
    }
}

fn session_page(f: &Form, c: &Ctx) -> gtk::Box {
    let p = page("Basic options for your Moriartty session");

    let g = group(&p, "Program to run");
    f.entry(
        &g,
        "Shell (blank = your login shell)",
        |s| s.shell.clone(),
        |s, v| s.shell = v,
    )
    .set_placeholder_text(Some("e.g. /usr/bin/zsh"));
    f.check(
        &g,
        "Run as a login shell",
        |s| s.login_shell,
        |s, v| s.login_shell = v,
    );
    f.entry(
        &g,
        "Command to run instead (blank = the shell)",
        |s| argv::join(&s.command),
        |s, v| s.command = argv::split(&v),
    )
    .set_placeholder_text(Some("e.g. ssh admin@prod.example.com"));
    note(
        &g,
        "Save a session with a command to open it in one go, from the ▾ next to the \
         + button or with -load. It is started directly, not through a shell: quotes \
         group words, but $variables, pipes and ; are not interpreted. New tabs and \
         windows opened from that session run it too.",
    );

    let g = group(&p, "Load, save or delete a stored session");
    note(
        &g,
        "\"Default Settings\" is used for every new window. Apply also saves the \
         settings to the highlighted session. Double-click a session to load it.",
    );
    if f.s.borrow().cli_options {
        note(
            &g,
            "This window was started with command-line options such as -log or -title, \
             so no session is highlighted and Apply only changes this window. Highlight \
             a session to save the settings there too, those options included.",
        );
    }
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Saved Sessions"));
    g.append(&name);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    // A click only highlights; loading (which replaces the settings being
    // edited) takes a double-click or Enter.
    list.set_activate_on_single_click(false);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&list));
    sw.set_hexpand(true);
    sw.set_min_content_height(180);
    sw.set_has_frame(true);
    let btns = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let load = gtk::Button::with_label("Load");
    let save = gtk::Button::with_label("Save");
    let delete = gtk::Button::with_label("Delete");
    btns.append(&load);
    btns.append(&save);
    btns.append(&delete);
    row.append(&sw);
    row.append(&btns);
    g.append(&row);
    fill_sessions(&list);

    {
        let (name, selected) = (name.clone(), c.selected.clone());
        list.connect_row_selected(move |_, row| {
            let n = row.and_then(row_name);
            if let Some(n) = &n {
                name.set_text(n);
            }
            *selected.borrow_mut() = n;
        });
    }
    // The window's own session, if it is still there.
    let initial = c.selected.borrow().clone();
    if !initial.is_some_and(|n| select_named(&list, &n)) {
        *c.selected.borrow_mut() = None;
    }
    {
        let (f, w) = (f.clone(), c.window.clone());
        list.connect_row_activated(move |_, row| {
            if let Some(n) = row_name(row) {
                load_into(&f, &w, &n);
            }
        });
    }
    {
        let (f, w, name, list) = (f.clone(), c.window.clone(), name.clone(), list.clone());
        load.connect_clicked(move |_| {
            let n = name.text().to_string();
            if !n.is_empty() && load_into(&f, &w, &n) {
                select_named(&list, &n);
            }
        });
    }
    {
        let (f, w, name, list) = (f.clone(), c.window.clone(), name.clone(), list.clone());
        save.connect_clicked(move |_| {
            let n = name.text().trim().to_string();
            if n.is_empty() {
                ui::message(
                    &w,
                    "Enter a session name first",
                    "Type a name in the Saved Sessions box.",
                );
                return;
            }
            let mut s = f.get();
            s.sanitize();
            if let Err(e) = check(&s) {
                ui::message(&w, "Invalid settings", &e);
                return;
            }
            match storage::save_session(&n, &s) {
                Ok(()) => {
                    fill_sessions(&list);
                    select_named(&list, &n);
                }
                Err(e) => ui::message(&w, "Could not save session", &e),
            }
        });
    }
    {
        let (w, name, list, selected) = (
            c.window.clone(),
            name.clone(),
            list.clone(),
            c.selected.clone(),
        );
        delete.connect_clicked(move |_| {
            let n = name.text().to_string();
            if n.is_empty() {
                return;
            }
            match storage::delete_session(&n) {
                Ok(()) => {
                    name.set_text("");
                    fill_sessions(&list);
                    // Nothing highlighted: Apply won't bring it back.
                    *selected.borrow_mut() = None;
                }
                Err(e) => ui::message(&w, "Could not delete session", &e),
            }
        });
    }
    if c.reconfig {
        note(
            &g,
            "Changes to the shell or command take effect when the shell is restarted.",
        );
    }

    let g = group(&p, "");
    f.choice(
        &g,
        "Close window on exit",
        |s| s.close_on_exit,
        |s, v| s.close_on_exit = v,
    );
    p
}

fn logging_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling session logging");
    let g = group(&p, "Session logging");
    f.choice(&g, "Session logging", |s| s.log_type, |s, v| s.log_type = v);
    f.entry(
        &g,
        "Log file name",
        |s| s.log_file.clone(),
        |s, v| s.log_file = v,
    );
    note(
        &g,
        "&Y year, &M month, &D day, &T time, && a literal &. \
         Log files are created with mode 0600 and symlinks are refused. \
         Logs can contain anything shown on screen — treat them as secrets.",
    );
    f.choice(
        &g,
        "What to do if the log file already exists",
        |s| s.log_exists,
        |s, v| s.log_exists = v,
    );
    f.check(
        &g,
        "Flush log file frequently",
        |s| s.log_flush,
        |s, v| s.log_flush = v,
    );
    f.check(
        &g,
        "Include header",
        |s| s.log_header,
        |s, v| s.log_header = v,
    );
    p
}

// ----------------------------------------------------------------- Terminal

fn terminal_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling the terminal emulation");
    let g = group(&p, "Set various terminal options");
    f.entry(
        &g,
        "Terminal type ($TERM)",
        |s| s.term_type.clone(),
        |s, v| s.term_type = v,
    );
    f.check(
        &g,
        "Enable blinking text",
        |s| s.blink_text,
        |s, v| s.blink_text = v,
    );
    f.check(
        &g,
        "Treat CJK ambiguous characters as wide",
        |s| s.cjk_ambig_wide,
        |s, v| s.cjk_ambig_wide = v,
    );
    p
}

fn keyboard_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling the effects of keys");
    let g = group(&p, "Change the sequences sent by");
    f.choice(
        &g,
        "The Backspace key",
        |s| s.backspace,
        |s, v| s.backspace = v,
    );

    let g = group(&p, "Shortcuts");
    note(
        &g,
        "Leave a box empty for the default keys, shown in grey. Separate several keys \
         with commas; \"none\" turns a shortcut off. Examples: Ctrl+Shift+T, Alt+F11, \
         Ctrl+PageUp, Ctrl+plus, Ctrl+comma.",
    );
    for def in keys::ACTIONS {
        let name = def.name;
        f.entry(
            &g,
            def.label,
            move |s| keys::setting_text(s.keys.get(name).map(Vec::as_slice)),
            move |s, v| keys::set_from_text(&mut s.keys, name, &v),
        )
        .set_placeholder_text(Some(&def.keys.join(", ")));
    }
    note(
        &g,
        "Tab switching keys only act with two or more tabs, and the prompt keys only \
         once the shell marks its prompts (shell integration, see the README); \
         otherwise the key goes to the program. Full screen's default keys only act \
         with \"Full screen on Alt-Enter\" (Behaviour page) turned on.",
    );
    p
}

fn bell_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling the terminal bell");
    let g = group(&p, "Set the style of bell");
    f.choice(
        &g,
        "Action to happen when a bell occurs",
        |s| s.bell,
        |s, v| s.bell = v,
    );
    let g = group(&p, "Control the bell overload behaviour");
    f.check(
        &g,
        "Bell is temporarily disabled when over-used",
        |s| s.bell_overload,
        |s, v| s.bell_overload = v,
    );
    f.number(
        &g,
        "Over-use means this many bells…",
        1,
        1000,
        |s| s.bell_overload_n,
        |s, v| s.bell_overload_n = v,
    );
    f.number(
        &g,
        "…in this many seconds",
        1,
        3600,
        |s| s.bell_overload_t,
        |s, v| s.bell_overload_t = v,
    );
    f.number(
        &g,
        "Seconds of silence required",
        1,
        3600,
        |s| s.bell_overload_s,
        |s, v| s.bell_overload_s = v,
    );
    p
}

fn features_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Enabling and disabling advanced terminal features");
    let g = group(&p, "Enable/disable advanced terminal features");
    f.check(
        &g,
        "Disable remote-controlled window title changing",
        |s| s.no_remote_wintitle,
        |s, v| s.no_remote_wintitle = v,
    );
    f.check(
        &g,
        "Ctrl+click opens web links (http/https only)",
        |s| s.clickable_urls,
        |s, v| s.clickable_urls = v,
    );
    f.check(
        &g,
        "Allow clickable hyperlinks (OSC 8)",
        |s| s.allow_hyperlinks,
        |s, v| s.allow_hyperlinks = v,
    );
    note(
        &g,
        "Always disabled for safety: answerback (^E), window title reporting and \
         clipboard writes by programs (OSC 52).",
    );

    let g = group(&p, "Shell integration");
    f.number(
        &g,
        "Tell me when a command that ran this many seconds ends in a background tab \
         (0 = never)",
        0,
        86_400,
        |s| s.notify_command_secs,
        |s, v| s.notify_command_secs = v,
    );
    note(
        &g,
        "Needs the shell integration script for bash, zsh or fish (see the README), \
         which also lets Ctrl+Shift+Up / Ctrl+Shift+Down jump between prompts. The \
         tab's name turns bold and the desktop shows a notice with the tab's number \
         and the exit status, never the command or its output.",
    );
    p
}

// ------------------------------------------------------------------- Window

fn window_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling Moriartty's window");
    let g = group(&p, "Set the size of the window");
    f.number(
        &g,
        "Columns",
        1,
        1000,
        |s| u32::from(s.columns),
        |s, v| s.columns = v as u16,
    );
    f.number(
        &g,
        "Rows",
        1,
        500,
        |s| u32::from(s.rows),
        |s, v| s.rows = v as u16,
    );
    let g = group(&p, "Control the scrollback in the window");
    f.number(
        &g,
        "Lines of scrollback",
        0,
        1_000_000,
        |s| s.scrollback,
        |s, v| s.scrollback = v,
    );
    f.check(
        &g,
        "Display scrollbar",
        |s| s.scrollbar,
        |s, v| s.scrollbar = v,
    );
    f.check(
        &g,
        "Reset scrollback on keypress",
        |s| s.scroll_on_key,
        |s, v| s.scroll_on_key = v,
    );
    f.check(
        &g,
        "Reset scrollback on display activity",
        |s| s.scroll_on_output,
        |s, v| s.scroll_on_output = v,
    );
    p
}

fn appearance_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Configure the appearance of Moriartty's window");
    let g = group(&p, "Adjust the use of the cursor");
    f.choice(&g, "Cursor appearance", |s| s.cursor, |s, v| s.cursor = v);
    f.check(
        &g,
        "Cursor blinks",
        |s| s.cursor_blink,
        |s, v| s.cursor_blink = v,
    );
    let g = group(&p, "Font settings");
    f.font(
        &g,
        "Font used in the terminal window",
        |s| s.font.clone(),
        |s, v| s.font = v,
    );
    f.number(
        &g,
        "Zoom (%) — also Ctrl+= / Ctrl+- / Ctrl+0",
        30,
        400,
        |s| s.font_zoom,
        |s, v| s.font_zoom = v,
    );
    let g = group(&p, "Adjust the use of the mouse pointer");
    f.check(
        &g,
        "Hide mouse pointer when typing in window",
        |s| s.hide_mouse,
        |s, v| s.hide_mouse = v,
    );
    let g = group(&p, "Adjust the window border");
    f.number(
        &g,
        "Gap between text and window edge",
        0,
        64,
        |s| u32::from(s.window_border),
        |s, v| s.window_border = v as u16,
    );
    let g = group(&p, "Background");
    f.number(
        &g,
        "Background opacity (%)",
        30,
        100,
        |s| s.background_opacity,
        |s, v| s.background_opacity = v,
    );
    note(
        &g,
        "Below 100 the desktop shows through the terminal. This needs a compositing desktop.",
    );
    p
}

fn behaviour_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Configure the behaviour of Moriartty's window");
    let g = group(&p, "");
    f.entry(
        &g,
        "Window title",
        |s| s.wintitle.clone(),
        |s, v| s.wintitle = v,
    );
    f.check(
        &g,
        "Warn before closing a window with a running program",
        |s| s.warn_on_close,
        |s, v| s.warn_on_close = v,
    );
    f.check(
        &g,
        "Full screen on Alt-Enter",
        |s| s.fullscreen_on_alt_enter,
        |s, v| s.fullscreen_on_alt_enter = v,
    );
    f.check(
        &g,
        "Show settings and menu buttons in the title bar",
        |s| s.title_buttons,
        |s, v| s.title_buttons = v,
    );
    note(
        &g,
        "Without the buttons, use Ctrl+Right-click for the menu and Ctrl+Shift+P for settings.",
    );
    p
}

fn selection_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling copy and paste");
    let g = group(&p, "Control use of mouse");
    f.choice(
        &g,
        "Action of mouse buttons",
        |s| s.mouse_action,
        |s, v| s.mouse_action = v,
    );
    note(
        &g,
        "Ctrl+Right-click always opens the menu. Shift+Right-click goes to the application.",
    );
    let g = group(&p, "Copy and paste");
    f.check(
        &g,
        "Also copy selected text to the clipboard",
        |s| s.copy_to_clipboard,
        |s, v| s.copy_to_clipboard = v,
    );
    note(
        &g,
        "Selected text always goes to the PRIMARY selection, which Middle-click \
         (and Right-click in Compromise mode) pastes. Ctrl+Shift+C or Ctrl+Insert \
         copies it to the clipboard. Clipboard managers keep whatever reaches the \
         clipboard, so copying every selection there is off by default.",
    );
    f.entry(
        &g,
        "Extra characters treated as part of a word",
        |s| s.word_chars.clone(),
        |s, v| s.word_chars = v,
    );
    f.check(
        &g,
        "Ask before pasting several lines or hidden characters",
        |s| s.confirm_multiline_paste,
        |s, v| s.confirm_multiline_paste = v,
    );
    f.check(
        &g,
        "Permit control characters in pasted text (unsafe)",
        |s| s.paste_controls,
        |s, v| s.paste_controls = v,
    );
    note(
        &g,
        "Pasted text is normally stripped of escape sequences and other control \
         characters, so text copied from a web page cannot run hidden commands.",
    );
    p
}

fn colours_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling use of colours");

    let g = group(&p, "Colour scheme");
    let follow = f.check(
        &g,
        "Follow the desktop's light or dark style (Moriartty Dark / Light)",
        |s| s.auto_colours,
        |s, v| s.auto_colours = v,
    );
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&gtk::Label::new(Some("Scheme")));
    let names: Vec<&str> = SCHEMES.iter().map(|(n, _)| *n).collect();
    let schemes = gtk::DropDown::from_strings(&names);
    schemes.set_hexpand(true);
    if let Some(i) = SCHEMES
        .iter()
        .position(|(_, cols)| *cols == f.s.borrow().colours)
    {
        schemes.set_selected(i as u32);
    }
    // Picking a scheme puts its colours into the settings at once: the
    // terminal shows them, Apply keeps them, Cancel goes back. (Connected
    // after the initial selection above, so opening the window changes
    // nothing.)
    {
        let f = f.clone();
        schemes.connect_selected_notify(move |dd| {
            let Some((_, colours)) = SCHEMES.get(dd.selected() as usize) else {
                return;
            };
            // The borrow ends before refresh(), whose widgets write back
            // into the settings.
            f.s.borrow_mut().colours = colours.clone();
            f.refresh();
        });
    }
    row.append(&schemes);
    g.append(&row);
    note(
        &g,
        "A scheme sets all the colours below, and the terminal shows it at once. Apply \
         keeps it; Cancel goes back to what you had. You can then change single colours.",
    );

    let g = group(&p, "General options for colour usage");
    f.check(
        &g,
        "Show bold text in a brighter colour",
        |s| s.bold_as_colour,
        |s, v| s.bold_as_colour = v,
    );
    let pickers = group(&p, "Adjust the precise colours Moriartty displays");
    f.colour(
        &pickers,
        "Default Foreground",
        |s| s.colours.foreground,
        |s, v| s.colours.foreground = v,
    );
    f.colour(
        &pickers,
        "Default Bold Foreground",
        |s| s.colours.bold_foreground,
        |s, v| s.colours.bold_foreground = v,
    );
    f.colour(
        &pickers,
        "Default Background",
        |s| s.colours.background,
        |s, v| s.colours.background = v,
    );
    f.colour(
        &pickers,
        "Cursor Text",
        |s| s.colours.cursor_text,
        |s, v| s.colours.cursor_text = v,
    );
    f.colour(
        &pickers,
        "Cursor Colour",
        |s| s.colours.cursor,
        |s, v| s.colours.cursor = v,
    );
    for (i, name) in ANSI_NAMES.iter().enumerate() {
        f.colour(
            &pickers,
            name,
            move |s| s.colours.ansi[i],
            move |s, v| s.colours.ansi[i] = v,
        );
    }

    // Picking colours by hand only applies when not following the desktop.
    let manual: [gtk::Widget; 2] = [row.upcast(), pickers.upcast()];
    let set_manual = move |follow: bool| {
        for w in &manual {
            w.set_sensitive(!follow);
        }
    };
    set_manual(f.s.borrow().auto_colours);
    follow.connect_toggled(move |b| set_manual(b.is_active()));
    p
}
