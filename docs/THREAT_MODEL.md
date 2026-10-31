# Threat model

What Moriartty defends against, how, and what it deliberately does not try
to defend against. Report problems as described in [SECURITY.md](../SECURITY.md).

## What is protected

The user's shell session: what they type, what they paste, what the
scrollback holds (often passwords, tokens and keys), the clipboard, and the
files Moriartty writes (settings and logs).

## Threats Moriartty defends against

### 1. Malicious terminal output

Anything a program prints is untrusted: `cat` of a downloaded file, `curl`
output, a compromised remote host behind `ssh`, a log file.

| Attack | Defence |
|---|---|
| Write to the clipboard (OSC 52) | Not implemented by VTE; locked in by `tests/evil_bytes.rs` |
| Read the clipboard or window title back into the shell (OSC 52 query, CSI 21 t) | Not answered; locked in by `tests/evil_bytes.rs` |
| Answerback (ENQ), echoing requests (DECRQSS, XTGETTCAP) to inject "typed" text | No answer contains attacker-chosen text; locked in by `tests/evil_bytes.rs` |
| Misleading window or tab title | Control, invisible and direction-changing characters stripped, length capped (`strip_controls`); remote titles can be turned off |
| Misleading link (OSC 8 hyperlink, lookalike domain) | OSC 8 off by default and always asks; international hosts ask and show the `xn--` form; only http(s); user names, backslashes and invisible characters in links refused (`matching.rs`) |
| Fake directory report (OSC 7) to make New Tab start in a hostile directory | OSC 7 only triggers a look; the directory always comes from the shell's real working directory (`/proc`) |
| Fake shell integration marks (OSC 133) to show a misleading notification | Marks are only read, never answered; at worst they misplace a prompt jump or report a command as finished. The notice is fixed text with the tab's number and a number for the exit status: nothing from the output is shown |
| Bell flood | Bell overload protection |
| Output flood: a program or remote host that never stops printing (0.9.9) | Output is read at low priority, below drawing and input, so the window keeps responding |

### 2. Malicious pastes

Text copied from a web page can hide commands.

| Attack | Defence |
|---|---|
| Escape sequences or control characters in the paste | Stripped (`paste.rs`), unless the user turns that off |
| Ending bracketed-paste mode early (`ESC [201~`) to run the rest | The terminator is always removed, even when control characters are allowed |
| Hidden newlines, invisible or direction-changing characters | Multi-line pastes and pastes with such characters ask first; the preview spells them out; Cancel is the default |
| A paste going around all this through VTE's own paste (Shift+Middle-click, Shift+Insert, Ctrl+Shift+Insert) (0.9.9) | Those clicks and keys never reach VTE: Shift+Middle-click takes the filtered path, and the keys do only what their shortcut says |
| A paste reaching a different shell than intended | Dropped if the shell was restarted or the tab closed meanwhile |

### 3. Data leaving the session

| Risk | Defence |
|---|---|
| Clipboard managers storing selected secrets | Selections go to PRIMARY only, unless the user opts in; Copy All asks for large scrollbacks |
| Core dumps or other processes reading the scrollback | Process is non-dumpable (no core files, no same-user ptrace) |
| Unnoticed logging | A tab being logged shows an indicator with the file name |
| A one-off `-log` staying on for later windows (0.9.9) | Apply doesn't save command-line options unless you highlight a session |
| Other programs on the session bus triggering Copy All, Paste or other menu actions (0.9.9) | Menu actions are kept in a private group, not exported on D-Bus, and a repeated activation over D-Bus does nothing |
| Launcher tokens passed to the shell | `XDG_ACTIVATION_TOKEN`, `DESKTOP_STARTUP_ID` and similar are removed |
| A dragged tab (0.9.8) showing its contents to whatever it is dropped on, or another program's drag posing as a tab | The drag carries no data, only an empty private type, and a tab is only accepted from a drag started in the same process |
| A session's warning colours lost when its tab moves to another window (0.9.8) | A tab only joins a window with the same settings; otherwise it gets a new window with its own |

### 4. Local file attacks

Another local user, or a program running as the user, plants files.

| Attack | Defence |
|---|---|
| Symlink to redirect a settings or log write | `O_NOFOLLOW`, atomic writes through an unpredictable temporary name |
| Files or directories owned by someone else | Refused |
| Readable settings or logs | Directory 0700, files 0600, repaired when found looser |
| FIFO or device as a log or settings file (hang or misuse) | Opened non-blocking, refused if not a regular file; a log is emptied only after the checks |
| Broken or hostile settings file locking the user out | Bad values fall back to safe ones and are reported; an unreadable Default Settings is moved aside, never overwritten |
| A folder name crafted to run commands through "Open in Moriartty" (0.9.7) | The file manager entries pass the folder as one argument to `-dir`, never through a shell, and `-dir` refuses anything that isn't a directory |
| A saved session's command (0.9.5) | Session files are yours alone (above), and a command only runs when you open that session; it is exec'd directly, never through a shell |

## Not defended against

* **A compromised user account.** Anything running as the user can already
  read their files, inject keystrokes through the desktop, or replace
  Moriartty itself.
* **The shell and the programs it runs.** Moriartty starts what it is told
  to; what those programs do is outside its control.
* **The desktop and display server**, GTK, VTE and the kernel. Moriartty
  relies on them, and on VTE's parser being memory safe.
* **Physical access** to an unlocked session.
* **Programs that the user lets in on purpose**: control characters allowed
  in pastes, OSC 8 hyperlinks turned on, logging to a shared location.
  These settings say they are risky.
* **Network attackers.** Moriartty makes no network connections; opening a
  link hands it to the desktop's default browser (through GIO, not the
  portal).
