#!/usr/bin/env bash
# Measure Moriartty against the performance budget in docs/PERFORMANCE.md.
# Run it in your desktop session: it opens windows.
#
# Usage:
#   ./perf-check.sh                        # the installed moriartty
#   ./perf-check.sh target/release/moriartty
#
# Uses built-in default settings (a temporary XDG_CONFIG_HOME), so your own
# settings neither affect the numbers nor get touched.
set -euo pipefail

BIN="${1:-$(command -v moriartty || true)}"
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    echo "No moriartty binary found; pass its path." >&2
    exit 1
fi

CFG=$(mktemp -d)
trap 'rm -rf "$CFG"' EXIT
export XDG_CONFIG_HOME="$CFG"

echo "Binary:  $BIN ($("$BIN" -version))"

# --- size ----------------------------------------------------------------
size=$(stat -c %s "$BIN")
if command -v strip >/dev/null; then
    tmp="$CFG/stripped"
    cp "$BIN" "$tmp"
    strip "$tmp"
    stripped=$(stat -c %s "$tmp")
else
    stripped="(strip not installed)"
fi
echo "Size:    $size bytes, stripped: $stripped"

# --- time to first output ---------------------------------------------------
# A program that prints at once and exits cleanly; with the default settings
# the window then closes by itself. Moriartty logs (MORIARTTY_DEBUG) how long
# after its start the first output arrived.
times=()
for _ in 1 2 3 4 5; do
    t=$(MORIARTTY_DEBUG=1 timeout 20 "$BIN" -e /bin/echo ready 2>&1 >/dev/null |
        sed -n 's/.*first output from the program, \([0-9][0-9]*\) ms after start.*/\1/p' |
        head -n1 || true)
    if [[ -n "$t" ]]; then
        times+=("$t")
    fi
done
if (( ${#times[@]} == 0 )); then
    echo "Startup: no measurement (is a desktop session running?)"
else
    median=$(printf '%s\n' "${times[@]}" | sort -n | sed -n "$(( (${#times[@]} + 1) / 2 ))p")
    verdict=$([[ "$median" -lt 150 ]] && echo "within budget (< 150 ms)" || echo "OVER BUDGET (150 ms)")
    echo "Startup: median ${median} ms to first output (runs: ${times[*]}) - $verdict"
fi

# --- memory per tab and idle wakeups ------------------------------------------
rss() { awk '/^VmRSS:/ {print $2}' "/proc/$1/status"; }
switches() { awk '/^(voluntary|nonvoluntary)_ctxt_switches:/ {n += $2} END {print n}' "/proc/$1/status"; }

echo
echo "A Moriartty window opens now. Leave it alone for a moment."
"$BIN" >/dev/null 2>&1 &
pid=$!
sleep 3
rss1=$(rss "$pid")

# Idle: nothing typed, no output. The process should hardly wake up.
s1=$(switches "$pid")
sleep 10
s2=$(switches "$pid")
echo "Idle:    $(( s2 - s1 )) wakeups in 10 s with one idle tab"

echo
read -r -p "Now press Ctrl+Shift+T nine times in that window (10 tabs), then press Enter here. "
sleep 2
rss2=$(rss "$pid")
s1=$(switches "$pid")
sleep 10
s2=$(switches "$pid")
echo "Idle:    $(( s2 - s1 )) wakeups in 10 s with ten idle tabs"
echo "Memory:  ${rss1} KiB with one tab, ${rss2} KiB with ten: about $(( (rss2 - rss1) / 9 )) KiB per extra tab"

kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
echo
echo "Record these in docs/PERFORMANCE.md (with the machine you measured on)."
