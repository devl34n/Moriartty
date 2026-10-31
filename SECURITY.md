# Security policy

Moriartty is a terminal emulator, so bugs in paste handling, escape-sequence
handling, file permissions or process handling can be security bugs.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately instead: open the
repository's **Security** tab and click **Report a vulnerability** (GitHub
private vulnerability reporting).

Please include the Moriartty version, your distribution, and steps to
reproduce. A proof of concept, such as text to paste or bytes to `cat`, helps
a lot.

## In scope

* Pasted text reaching the shell with control or escape sequences, or
  breaking out of bracketed-paste mode.
* Output from a program causing anything beyond what is displayed on screen:
  clipboard writes, title reports, answerback, file access.
* Settings or log files being created with broken permissions, through
  symlinks, or in places owned by other users.
* Crashes or memory-safety issues, including anything in `src/sys.rs`, the
  only module with `unsafe` code.

## Supported versions

Only the latest release gets security fixes.
