//! Small data-binding layer for the settings dialog: every widget writes
//! straight into a shared `Settings`, and `Form::refresh` pushes a (loaded)
//! `Settings` back into every widget.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib, pango};

use crate::config::{Choice, Rgb, Settings};

pub type Shared = Rc<RefCell<Settings>>;
type Refresher = Box<dyn Fn(&Settings)>;
type Callback = Box<dyn Fn()>;

#[derive(Clone)]
pub struct Form {
    pub s: Shared,
    refreshers: Rc<RefCell<Vec<Refresher>>>,
    refreshing: Rc<Cell<bool>>,
    changed: Changed,
}

/// Callbacks run after the user changes a value (live preview). While
/// `refresh` pushes values into the widgets they are held back, and run
/// once at the end.
#[derive(Clone)]
struct Changed {
    callbacks: Rc<RefCell<Vec<Callback>>>,
    refreshing: Rc<Cell<bool>>,
}

impl Changed {
    fn fire(&self) {
        if !self.refreshing.get() {
            for f in self.callbacks.borrow().iter() {
                f();
            }
        }
    }
}

pub fn page(title: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b.set_margin_start(12);
    b.set_margin_end(12);
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(title)));
    l.set_xalign(0.0);
    b.append(&l);
    b
}

pub fn group(parent: &gtk::Box, title: &str) -> gtk::Box {
    let f = gtk::Frame::new(Some(title));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.set_margin_top(8);
    b.set_margin_bottom(8);
    b.set_margin_start(8);
    b.set_margin_end(8);
    f.set_child(Some(&b));
    parent.append(&f);
    b
}

pub fn note(parent: &gtk::Box, text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.add_css_class("dim-label");
    parent.append(&l);
    l
}

fn labelled(parent: &gtk::Box, label: &str, w: &impl IsA<gtk::Widget>, expand: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = gtk::Label::new(Some(label));
    l.set_xalign(0.0);
    l.set_width_chars(30);
    l.set_max_width_chars(30);
    l.set_wrap(true);
    row.append(&l);
    w.set_hexpand(expand);
    if !expand {
        w.set_halign(gtk::Align::Start);
    }
    row.append(w);
    parent.append(&row);
    row
}

pub fn rgba(c: Rgb) -> gdk::RGBA {
    gdk::RGBA::new(
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
        1.0,
    )
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl Form {
    pub fn new(s: Settings) -> Self {
        let refreshing: Rc<Cell<bool>> = Rc::default();
        Form {
            s: Rc::new(RefCell::new(s)),
            refreshers: Rc::default(),
            refreshing: refreshing.clone(),
            changed: Changed {
                callbacks: Rc::default(),
                refreshing,
            },
        }
    }

    /// Drop every widget binding, so a closed dialog can be freed: the
    /// bindings and the widgets' signal handlers hold on to each other.
    pub fn clear(&self) {
        if let Ok(mut r) = self.refreshers.try_borrow_mut() {
            r.clear();
        }
        if let Ok(mut c) = self.changed.callbacks.try_borrow_mut() {
            c.clear();
        }
    }

    /// Call `f` after every change the user makes.
    pub fn on_change(&self, f: impl Fn() + 'static) {
        self.changed.callbacks.borrow_mut().push(Box::new(f));
    }

    pub fn get(&self) -> Settings {
        self.s.borrow().clone()
    }

    pub fn set_all(&self, s: Settings) {
        *self.s.borrow_mut() = s;
        self.refresh();
    }

    pub fn refresh(&self) {
        let snapshot = self.get();
        self.refreshing.set(true);
        for r in self.refreshers.borrow().iter() {
            r(&snapshot);
        }
        self.refreshing.set(false);
        self.changed.fire();
    }

    fn on_refresh(&self, f: impl Fn(&Settings) + 'static) {
        self.refreshers.borrow_mut().push(Box::new(f));
    }

    pub fn entry(
        &self,
        parent: &gtk::Box,
        label: &str,
        get: impl Fn(&Settings) -> String + 'static,
        set: impl Fn(&mut Settings, String) + 'static,
    ) -> gtk::Entry {
        let e = gtk::Entry::new();
        labelled(parent, label, &e, true);
        let (s, changed) = (self.s.clone(), self.changed.clone());
        e.connect_changed(move |e| {
            set(&mut s.borrow_mut(), e.text().to_string());
            changed.fire();
        });
        let e2 = e.clone();
        self.on_refresh(move |st| {
            let v = get(st);
            if e2.text().as_str() != v {
                e2.set_text(&v);
            }
        });
        e
    }

    pub fn check(
        &self,
        parent: &gtk::Box,
        label: &str,
        get: impl Fn(&Settings) -> bool + 'static,
        set: impl Fn(&mut Settings, bool) + 'static,
    ) -> gtk::CheckButton {
        let c = gtk::CheckButton::with_label(label);
        parent.append(&c);
        let (s, changed) = (self.s.clone(), self.changed.clone());
        c.connect_toggled(move |c| {
            set(&mut s.borrow_mut(), c.is_active());
            changed.fire();
        });
        let c2 = c.clone();
        self.on_refresh(move |st| {
            let v = get(st);
            if c2.is_active() != v {
                c2.set_active(v);
            }
        });
        c
    }

    pub fn number(
        &self,
        parent: &gtk::Box,
        label: &str,
        min: u32,
        max: u32,
        get: impl Fn(&Settings) -> u32 + 'static,
        set: impl Fn(&mut Settings, u32) + 'static,
    ) -> gtk::SpinButton {
        let sp = gtk::SpinButton::with_range(f64::from(min), f64::from(max), 1.0);
        sp.set_digits(0);
        labelled(parent, label, &sp, false);
        let (s, changed) = (self.s.clone(), self.changed.clone());
        sp.connect_value_changed(move |sp| {
            let v = sp.value().round().clamp(f64::from(min), f64::from(max)) as u32;
            set(&mut s.borrow_mut(), v);
            changed.fire();
        });
        let sp2 = sp.clone();
        self.on_refresh(move |st| {
            let v = f64::from(get(st));
            if sp2.value() != v {
                sp2.set_value(v);
            }
        });
        sp
    }

    pub fn choice<T: Choice>(
        &self,
        parent: &gtk::Box,
        label: &str,
        get: impl Fn(&Settings) -> T + 'static,
        set: impl Fn(&mut Settings, T) + 'static,
    ) -> gtk::DropDown {
        let labels: Vec<&str> = T::all().iter().map(|v| v.label()).collect();
        let dd = gtk::DropDown::from_strings(&labels);
        labelled(parent, label, &dd, true);
        let (s, changed) = (self.s.clone(), self.changed.clone());
        let refreshing = self.refreshing.clone();
        dd.connect_selected_notify(move |dd| {
            if refreshing.get() {
                return;
            }
            if let Some(v) = T::all().get(dd.selected() as usize) {
                set(&mut s.borrow_mut(), *v);
                changed.fire();
            }
        });
        let dd2 = dd.clone();
        self.on_refresh(move |st| {
            let cur = get(st);
            let idx = T::all().iter().position(|v| *v == cur).unwrap_or(0) as u32;
            if dd2.selected() != idx {
                dd2.set_selected(idx);
            }
        });
        dd
    }

    pub fn colour(
        &self,
        parent: &gtk::Box,
        label: &str,
        get: impl Fn(&Settings) -> Rgb + 'static,
        set: impl Fn(&mut Settings, Rgb) + 'static,
    ) {
        let dialog = gtk::ColorDialog::new();
        dialog.set_with_alpha(false);
        let b = gtk::ColorDialogButton::new(Some(dialog));
        labelled(parent, label, &b, false);
        let (s, changed) = (self.s.clone(), self.changed.clone());
        b.connect_rgba_notify(move |b| {
            let c = b.rgba();
            set(
                &mut s.borrow_mut(),
                [to8(c.red()), to8(c.green()), to8(c.blue())],
            );
            changed.fire();
        });
        let b2 = b.clone();
        self.on_refresh(move |st| b2.set_rgba(&rgba(get(st))));
    }

    pub fn font(
        &self,
        parent: &gtk::Box,
        label: &str,
        get: impl Fn(&Settings) -> String + 'static,
        set: impl Fn(&mut Settings, String) + 'static,
    ) {
        let b = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
        labelled(parent, label, &b, true);
        let (s, changed) = (self.s.clone(), self.changed.clone());
        b.connect_font_desc_notify(move |b| {
            if let Some(d) = b.font_desc() {
                set(&mut s.borrow_mut(), d.to_str().to_string());
                changed.fire();
            }
        });
        let b2 = b.clone();
        self.on_refresh(move |st| {
            let want = get(st);
            let cur = b2.font_desc().map(|d| d.to_str().to_string());
            if cur.as_deref() != Some(want.as_str()) {
                b2.set_font_desc(&pango::FontDescription::from_string(&want));
            }
        });
    }
}
