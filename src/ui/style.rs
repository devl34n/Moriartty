//! Following the desktop's appearance: light or dark style and high
//! contrast. Sources, best first:
//!  1. the XDG Settings portal (GNOME, KDE and most others);
//!  2. GNOME's own setting, read directly. The portal refuses to answer
//!     non-dumpable processes (it can't open /proc/PID/root), and Moriartty
//!     is non-dumpable on purpose, so on GNOME this is the usual source;
//!  3. the GTK theme: a theme named "…-dark", or GTK's prefer-dark setting.
//!     Cinnamon, MATE and XFCE switch to dark that way.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::{gio, glib};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";

type Listener = Box<dyn Fn() -> bool>;

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
    /// The desktop publishes a colour scheme (portal or GNOME), so SCHEME
    /// is its real setting.
    static HAS_SCHEME: Cell<bool> = const { Cell::new(false) };
    /// `color-scheme`: 0 no preference, 1 dark, 2 light.
    static SCHEME: Cell<u32> = const { Cell::new(0) };
    /// `contrast`: 0 normal, 1 high.
    static CONTRAST: Cell<u32> = const { Cell::new(0) };
    static LISTENERS: RefCell<Vec<Listener>> = RefCell::new(Vec::new());
    /// GNOME's settings, kept alive for their change notifications.
    static GNOME: RefCell<Option<gio::Settings>> = const { RefCell::new(None) };
}

pub fn dark() -> bool {
    if HAS_SCHEME.with(Cell::get) {
        // Only an explicit dark preference means dark: GNOME reports its
        // light style as "default" (0), not as light (2).
        return SCHEME.with(Cell::get) == 1;
    }
    gtk_theme_is_dark()
}

/// Without a published scheme: the GTK theme's own idea.
fn gtk_theme_is_dark() -> bool {
    let Some(s) = gtk::Settings::default() else {
        return true;
    };
    s.is_gtk_application_prefer_dark_theme()
        || s.gtk_theme_name()
            .is_some_and(|n| n.to_lowercase().contains("dark"))
}

pub fn high_contrast() -> bool {
    CONTRAST.with(Cell::get) == 1
}

/// Call `f` whenever the desktop's appearance changes, for as long as it
/// returns true.
pub fn on_change(f: impl Fn() -> bool + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

/// Read the current appearance and watch for changes. The portal read is
/// one quick D-Bus call, so the first window already has the right
/// colours; the portal is never started just for this.
#[allow(deprecated)] // signal_subscribe: still the simplest way
pub fn init() {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    // Without a published scheme, a change of GTK theme is the desktop
    // changing. (With one, GTK's settings are what apply_gtk_theme sets,
    // so reacting to them could loop.)
    if let Some(settings) = gtk::Settings::default() {
        for prop in ["gtk-theme-name", "gtk-application-prefer-dark-theme"] {
            settings.connect_notify_local(Some(prop), |_, _| {
                if !HAS_SCHEME.with(Cell::get) {
                    changed();
                }
            });
        }
    }
    if let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
        for key in ["color-scheme", "contrast"] {
            let v = read(&bus, key);
            super::debug(format!("style: portal {key} = {v:?}"));
            if let Some(v) = v {
                store(key, v);
            }
        }
        let _ = bus.signal_subscribe(
            Some(PORTAL),
            Some(SETTINGS),
            Some("SettingChanged"),
            Some(PATH),
            Some(APPEARANCE),
            gio::DBusSignalFlags::NONE,
            |_, _, _, _, _, params| {
                // (namespace, key, value). Checked, not assumed: release
                // builds abort on a panic, and this comes from another
                // process.
                if params.type_().as_str() != "(ssv)" {
                    return;
                }
                let key = params.child_value(1).get::<String>();
                let value = unwrap(params.child_value(2)).get::<u32>();
                if let (Some(key), Some(value)) = (key, value) {
                    if store(&key, value) {
                        changed();
                    }
                }
            },
        );
    }
    if !HAS_SCHEME.with(Cell::get) {
        let watching = watch_gnome();
        super::debug(format!("style: GNOME color-scheme watched: {watching}"));
    }
    apply_gtk_theme();
}

/// One appearance setting from the portal: `ReadOne`, or the older `Read`,
/// which wraps the value in an extra variant.
fn read(bus: &gio::DBusConnection, key: &str) -> Option<u32> {
    for method in ["ReadOne", "Read"] {
        let reply = bus.call_sync(
            Some(PORTAL),
            PATH,
            SETTINGS,
            method,
            Some(&(APPEARANCE.to_string(), key.to_string()).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            200,
            None::<&gio::Cancellable>,
        );
        if let Ok(reply) = reply {
            if reply.type_().as_str() != "(v)" {
                return None;
            }
            return unwrap(reply.child_value(0)).get::<u32>();
        }
    }
    None
}

/// GNOME's `color-scheme`, read and watched directly (dconf, no D-Bus
/// round trip). False when this isn't GNOME's settings schema.
fn watch_gnome() -> bool {
    let Some(source) = gio::SettingsSchemaSource::default() else {
        return false;
    };
    // Settings::new aborts on a missing schema or key, so check first.
    let Some(schema) = source.lookup(GNOME_INTERFACE, true) else {
        return false;
    };
    if !schema.has_key("color-scheme") {
        return false;
    }
    let settings = gio::Settings::new(GNOME_INTERFACE);
    store("color-scheme", gnome_scheme(&settings));
    settings.connect_changed(Some("color-scheme"), |s, _| {
        if store("color-scheme", gnome_scheme(s)) {
            changed();
        }
    });
    GNOME.with(|g| *g.borrow_mut() = Some(settings));
    true
}

/// GNOME's values mapped to the portal's: "default" is GNOME's light style.
fn gnome_scheme(s: &gio::Settings) -> u32 {
    match s.string("color-scheme").as_str() {
        "prefer-dark" => 1,
        "prefer-light" => 2,
        _ => 0,
    }
}

fn unwrap(mut v: glib::Variant) -> glib::Variant {
    while let Some(inner) = v.as_variant() {
        v = inner;
    }
    v
}

/// Remember a value; true if it changed.
fn store(key: &str, value: u32) -> bool {
    let cell = match key {
        "color-scheme" => {
            if !HAS_SCHEME.with(|p| p.replace(true)) {
                SCHEME.with(|c| c.set(value));
                return true;
            }
            &SCHEME
        }
        "contrast" => &CONTRAST,
        _ => return false,
    };
    cell.with(|c| c.replace(value)) != value
}

/// Dialogs and menus follow the desktop too. Without a published scheme,
/// the GTK theme already is the desktop's choice and is left alone. Only
/// set when it really changes, so nothing is restyled for nothing.
fn apply_gtk_theme() {
    if !HAS_SCHEME.with(Cell::get) {
        return;
    }
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let dark = SCHEME.with(Cell::get) == 1;
    if settings.is_gtk_application_prefer_dark_theme() != dark {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

fn changed() {
    super::debug(format!(
        "style: changed (scheme known: {}, color-scheme: {}, contrast: {})",
        HAS_SCHEME.with(Cell::get),
        SCHEME.with(Cell::get),
        CONTRAST.with(Cell::get)
    ));
    apply_gtk_theme();
    // Run on a taken list, so a listener may register new ones.
    let list = LISTENERS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    let kept: Vec<Listener> = list.into_iter().filter(|f| f()).collect();
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        let added = std::mem::take(&mut *l);
        *l = kept;
        l.extend(added);
    });
}
