# Moriartty: "Open in Moriartty" in GNOME Files (Nautilus 43 and newer,
# extension API 4.0 or 4.1).
#
# Loaded by nautilus-python (Fedora: nautilus-python, Debian:
# python3-nautilus); without it this file does nothing. Adds the item to the
# menu of a single selected folder and of the empty space in a folder. It
# runs `moriartty -dir FOLDER` with the folder as one argument and no shell,
# so no folder name can run anything.

import gi

# Older Nautilus has extension API 4.0, newer (51 on Fedora 45) 4.1, with
# the same calls.
# nautilus-python has already loaded one of them: asking for the other is
# an error, so try the newer one first.
try:
    gi.require_version("Nautilus", "4.1")
except ValueError:
    gi.require_version("Nautilus", "4.0")

from gi.repository import GLib, GObject, Gio, Nautilus  # noqa: E402


def _local_dir(info):
    """The local path of a folder; None for files and remote places."""
    if info is None or not info.is_directory():
        return None
    location = info.get_location()
    return location.get_path() if location is not None else None


def _open(_item, path):
    # Never let an error reach Nautilus: a missing Moriartty must not break
    # the file manager.
    try:
        Gio.Subprocess.new(["moriartty", "-dir", path], Gio.SubprocessFlags.NONE)
    except GLib.Error as e:
        print(f"moriartty-nautilus: cannot start Moriartty: {e.message}")


def _item(name, path):
    item = Nautilus.MenuItem(
        name=name,
        label="Open in Moriartty",
        tip="Open a Moriartty terminal in this folder",
    )
    item.connect("activate", _open, path)
    return item


class MoriarttyExtension(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, files):
        if len(files) != 1:
            return []
        path = _local_dir(files[0])
        return [_item("Moriartty::OpenFolder", path)] if path else []

    def get_background_items(self, current_folder):
        path = _local_dir(current_folder)
        return [_item("Moriartty::OpenHere", path)] if path else []
