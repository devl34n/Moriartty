# Moriartty shell integration for fish.
#
# Marks where each prompt starts and when each command starts and ends, with
# invisible OSC 133 escape sequences. Moriartty uses them to jump between
# prompts (Ctrl+Shift+Up / Ctrl+Shift+Down) and to tell you when a long
# command ends in a background tab. Nothing leaves the terminal. (fish 4
# sends prompt marks itself; this adds the command marks.)
#
# Add this to ~/.config/fish/config.fish:
#
#   test -r /usr/share/moriartty/shell-integration/moriartty.fish
#   and source /usr/share/moriartty/shell-integration/moriartty.fish

# Interactive shells in Moriartty only, and only once.
if status is-interactive; and test "$TERM_PROGRAM" = moriartty; and not set -q __moriartty_si
    set -g __moriartty_si 1

    function __moriartty_prompt --on-event fish_prompt
        printf '\e]133;A\a'
    end

    function __moriartty_preexec --on-event fish_preexec
        printf '\e]133;C\a'
    end

    function __moriartty_postexec --on-event fish_postexec
        printf '\e]133;D;%s\a' $status
    end
end
