//! The PuTTY/pterm-style settings window: category tree on the left, option
//! pages on the right, saved sessions on the Session page.

use std::rc::Rc;

use gtk::prelude::*;

use super::form::{group, note, page, Form};
use crate::config::*;
use crate::{storage, ui};

pub enum Mode {
    New,
    Reconfigure(Rc<dyn Fn(Settings)>),
}

struct Ctx {
    window: gtk::ApplicationWindow,
    open: Rc<dyn Fn()>,
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

pub fn show(app: &gtk::Application, initial: Settings, mode: Mode) {
    let reconfig = matches!(mode, Mode::Reconfigure(_));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .icon_name(crate::APP_ID)
        .title(if reconfig { "Moriartty Reconfiguration" } else { "Moriartty Configuration" })
        .default_width(780)
        .default_height(620)
        .build();
    let form = Form::new(initial);

    let open: Rc<dyn Fn()> = {
        let form = form.clone();
        let window = window.clone();
        let app = app.clone();
        Rc::new(move || {
            let mut s = form.get();
            s.sanitize();
            if let Err(e) = s.validate() {
                ui::message(&window, "Invalid settings", &e);
                return;
            }
            match &mode {
                Mode::New => ui::terminal::open(&app, s, None, None),
                Mode::Reconfigure(apply) => apply(s),
            }
            window.close();
        })
    };

    let ctx = Ctx {
        window: window.clone(),
        open: open.clone(),
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
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
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
    f.entry(&g, "Shell (blank = your login shell)", |s| s.shell.clone(), |s, v| s.shell = v)
        .set_placeholder_text(Some("e.g. /usr/bin/zsh"));
    f.check(&g, "Run as a login shell", |s| s.login_shell, |s, v| s.login_shell = v);

    let g = group(&p, "Load, save or delete a stored session");
    note(&g, "\"Default Settings\" is used for every new window.");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Saved Sessions"));
    g.append(&name);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
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
        let name = name.clone();
        list.connect_row_selected(move |_, row| {
            if let Some(n) = row.and_then(row_name) {
                name.set_text(&n);
            }
        });
    }
    {
        let (f, w, open) = (f.clone(), c.window.clone(), c.open.clone());
        list.connect_row_activated(move |_, row| {
            if let Some(n) = row_name(row) {
                if load_into(&f, &w, &n) {
                    open();
                }
            }
        });
    }
    {
        let (f, w, name) = (f.clone(), c.window.clone(), name.clone());
        load.connect_clicked(move |_| {
            let n = name.text().to_string();
            if !n.is_empty() {
                load_into(&f, &w, &n);
            }
        });
    }
    {
        let (f, w, name, list) = (f.clone(), c.window.clone(), name.clone(), list.clone());
        save.connect_clicked(move |_| {
            let n = name.text().trim().to_string();
            if n.is_empty() {
                ui::message(&w, "Enter a session name first", "Type a name in the Saved Sessions box.");
                return;
            }
            let mut s = f.get();
            s.sanitize();
            match storage::save_session(&n, &s) {
                Ok(()) => fill_sessions(&list),
                Err(e) => ui::message(&w, "Could not save session", &e),
            }
        });
    }
    {
        let (w, name, list) = (c.window.clone(), name.clone(), list.clone());
        delete.connect_clicked(move |_| {
            let n = name.text().to_string();
            if n.is_empty() {
                return;
            }
            match storage::delete_session(&n) {
                Ok(()) => {
                    name.set_text("");
                    fill_sessions(&list);
                }
                Err(e) => ui::message(&w, "Could not delete session", &e),
            }
        });
    }
    if c.reconfig {
        note(&g, "Changes to the shell take effect when the shell is restarted.");
    }

    let g = group(&p, "");
    f.choice(&g, "Close window on exit", |s| s.close_on_exit, |s, v| s.close_on_exit = v);
    p
}

fn logging_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling session logging");
    let g = group(&p, "Session logging");
    f.choice(&g, "Session logging", |s| s.log_type, |s, v| s.log_type = v);
    f.entry(&g, "Log file name", |s| s.log_file.clone(), |s, v| s.log_file = v);
    note(
        &g,
        "&Y year, &M month, &D day, &T time, && a literal &. \
         Log files are created with mode 0600 and symlinks are refused. \
         Logs can contain anything shown on screen — treat them as secrets.",
    );
    f.choice(&g, "What to do if the log file already exists", |s| s.log_exists, |s, v| s.log_exists = v);
    f.check(&g, "Flush log file frequently", |s| s.log_flush, |s, v| s.log_flush = v);
    f.check(&g, "Include header", |s| s.log_header, |s, v| s.log_header = v);
    p
}

// ----------------------------------------------------------------- Terminal

fn terminal_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling the terminal emulation");
    let g = group(&p, "Set various terminal options");
    f.entry(&g, "Terminal type ($TERM)", |s| s.term_type.clone(), |s, v| s.term_type = v);
    f.check(&g, "Enable blinking text", |s| s.blink_text, |s, v| s.blink_text = v);
    f.check(&g, "Treat CJK ambiguous characters as wide", |s| s.cjk_ambig_wide, |s, v| s.cjk_ambig_wide = v);
    p
}

fn keyboard_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling the effects of keys");
    let g = group(&p, "Change the sequences sent by");
    f.choice(&g, "The Backspace key", |s| s.backspace, |s, v| s.backspace = v);
    note(
        &g,
        "Shortcuts: Ctrl+Shift+C / Ctrl+Insert copy, Ctrl+Shift+V / Shift+Insert paste, \
         Ctrl+Shift+N new window.",
    );
    p
}

fn bell_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling the terminal bell");
    let g = group(&p, "Set the style of bell");
    f.choice(&g, "Action to happen when a bell occurs", |s| s.bell, |s, v| s.bell = v);
    let g = group(&p, "Control the bell overload behaviour");
    f.check(&g, "Bell is temporarily disabled when over-used", |s| s.bell_overload, |s, v| s.bell_overload = v);
    f.number(&g, "Over-use means this many bells…", 1, 1000, |s| s.bell_overload_n, |s, v| s.bell_overload_n = v);
    f.number(&g, "…in this many seconds", 1, 3600, |s| s.bell_overload_t, |s, v| s.bell_overload_t = v);
    f.number(&g, "Seconds of silence required", 1, 3600, |s| s.bell_overload_s, |s, v| s.bell_overload_s = v);
    p
}

fn features_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Enabling and disabling advanced terminal features");
    let g = group(&p, "Enable/disable advanced terminal features");
    f.check(&g, "Disable remote-controlled window title changing", |s| s.no_remote_wintitle, |s, v| s.no_remote_wintitle = v);
    f.check(&g, "Allow clickable hyperlinks (OSC 8)", |s| s.allow_hyperlinks, |s, v| s.allow_hyperlinks = v);
    note(
        &g,
        "Always disabled for safety: answerback (^E), window title reporting and \
         clipboard writes by programs (OSC 52).",
    );
    p
}

// ------------------------------------------------------------------- Window

fn window_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling Moriartty's window");
    let g = group(&p, "Set the size of the window");
    f.number(&g, "Columns", 1, 1000, |s| u32::from(s.columns), |s, v| s.columns = v as u16);
    f.number(&g, "Rows", 1, 500, |s| u32::from(s.rows), |s, v| s.rows = v as u16);
    let g = group(&p, "Control the scrollback in the window");
    f.number(&g, "Lines of scrollback", 0, 1_000_000, |s| s.scrollback, |s, v| s.scrollback = v);
    f.check(&g, "Display scrollbar", |s| s.scrollbar, |s, v| s.scrollbar = v);
    f.check(&g, "Reset scrollback on keypress", |s| s.scroll_on_key, |s, v| s.scroll_on_key = v);
    f.check(&g, "Reset scrollback on display activity", |s| s.scroll_on_output, |s, v| s.scroll_on_output = v);
    p
}

fn appearance_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Configure the appearance of Moriartty's window");
    let g = group(&p, "Adjust the use of the cursor");
    f.choice(&g, "Cursor appearance", |s| s.cursor, |s, v| s.cursor = v);
    f.check(&g, "Cursor blinks", |s| s.cursor_blink, |s, v| s.cursor_blink = v);
    let g = group(&p, "Font settings");
    f.font(&g, "Font used in the terminal window", |s| s.font.clone(), |s, v| s.font = v);
    let g = group(&p, "Adjust the use of the mouse pointer");
    f.check(&g, "Hide mouse pointer when typing in window", |s| s.hide_mouse, |s, v| s.hide_mouse = v);
    let g = group(&p, "Adjust the window border");
    f.number(&g, "Gap between text and window edge", 0, 64, |s| u32::from(s.window_border), |s, v| s.window_border = v as u16);
    p
}

fn behaviour_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Configure the behaviour of Moriartty's window");
    let g = group(&p, "");
    f.entry(&g, "Window title", |s| s.wintitle.clone(), |s, v| s.wintitle = v);
    f.check(&g, "Warn before closing a window with a running program", |s| s.warn_on_close, |s, v| s.warn_on_close = v);
    f.check(&g, "Full screen on Alt-Enter", |s| s.fullscreen_on_alt_enter, |s, v| s.fullscreen_on_alt_enter = v);
    p
}

fn selection_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling copy and paste");
    let g = group(&p, "Control use of mouse");
    f.choice(&g, "Action of mouse buttons", |s| s.mouse_action, |s, v| s.mouse_action = v);
    note(&g, "Ctrl+Right-click always opens the menu. Hold Shift to send clicks to the application.");
    let g = group(&p, "Copy and paste");
    f.check(&g, "Copy selected text to the clipboard automatically", |s| s.copy_on_select, |s, v| s.copy_on_select = v);
    f.entry(&g, "Extra characters treated as part of a word", |s| s.word_chars.clone(), |s, v| s.word_chars = v);
    f.check(&g, "Ask before pasting text that contains line breaks", |s| s.confirm_multiline_paste, |s, v| s.confirm_multiline_paste = v);
    f.check(&g, "Permit control characters in pasted text (unsafe)", |s| s.paste_controls, |s, v| s.paste_controls = v);
    note(
        &g,
        "Pasted text is normally stripped of escape sequences and other control \
         characters, so text copied from a web page cannot run hidden commands.",
    );
    p
}

fn colours_page(f: &Form, _c: &Ctx) -> gtk::Box {
    let p = page("Options controlling use of colours");
    let g = group(&p, "General options for colour usage");
    f.check(&g, "Show bold text in a brighter colour", |s| s.bold_as_colour, |s, v| s.bold_as_colour = v);
    let g = group(&p, "Adjust the precise colours Moriartty displays");
    f.colour(&g, "Default Foreground", |s| s.colours.foreground, |s, v| s.colours.foreground = v);
    f.colour(&g, "Default Bold Foreground", |s| s.colours.bold_foreground, |s, v| s.colours.bold_foreground = v);
    f.colour(&g, "Default Background", |s| s.colours.background, |s, v| s.colours.background = v);
    f.colour(&g, "Cursor Text", |s| s.colours.cursor_text, |s, v| s.colours.cursor_text = v);
    f.colour(&g, "Cursor Colour", |s| s.colours.cursor, |s, v| s.colours.cursor = v);
    for (i, name) in ANSI_NAMES.iter().enumerate() {
        f.colour(&g, name, move |s| s.colours.ansi[i], move |s, v| s.colours.ansi[i] = v);
    }
    let reset = gtk::Button::with_label("Reset to PuTTY defaults");
    reset.set_halign(gtk::Align::Start);
    {
        let f = f.clone();
        reset.connect_clicked(move |_| {
            f.s.borrow_mut().colours = Colours::default();
            f.refresh();
        });
    }
    g.append(&reset);
    p
}
