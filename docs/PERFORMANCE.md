# Performance budget

Moriartty should start fast, stay small and cost nothing while idle. These
are measured with `./perf-check.sh` before each release, not guessed.

| What | Budget | How it is measured |
|---|---|---|
| Time to first output | under 150 ms | Moriartty's start to the first byte from `-e /bin/echo ready`, median of 5 runs (`MORIARTTY_DEBUG` logs it) |
| Idle wakeups | close to 0 in 10 s, with 1 or 10 idle tabs | context switches of the Moriartty process while nothing happens |
| Memory per extra tab | recorded; investigated if it grows by more than 20 % between releases | resident memory with 1 tab and with 10 |
| Binary size | recorded; investigated if it grows by more than 20 % between releases | size of the stripped binary |

"Time to first output" measures Moriartty itself: GTK, VTE, the window and
the pty. How long your shell then takes to show a prompt depends on your
shell's start-up files, not on Moriartty.

Idle tabs do no work: a tab's name ("program: directory") is only re-read
shortly after something happens in it (output, Enter, or the shell reporting
a new directory), never on a timer.

## Results

Fill in one row per release, with the machine it was measured on.

| Version | Machine | First output | Idle wakeups (1 / 10 tabs) | Per extra tab | Stripped size |
|---|---|---|---|---|---|
| 0.9.0 | | | | | |
