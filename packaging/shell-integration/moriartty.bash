# Moriartty shell integration for bash (4.4 or newer).
#
# Marks where each prompt starts and when each command starts and ends, with
# invisible OSC 133 escape sequences. Moriartty uses them to jump between
# prompts (Ctrl+Shift+Up / Ctrl+Shift+Down) and to tell you when a long
# command ends in a background tab. Nothing leaves the terminal.
#
# Add this to the end of ~/.bashrc:
#
#   [ -r /usr/share/moriartty/shell-integration/moriartty.bash ] &&
#       . /usr/share/moriartty/shell-integration/moriartty.bash

# Interactive shells in Moriartty only, and only once.
if [[ $- == *i* && ${TERM_PROGRAM-} == moriartty && -z ${__moriartty_si-} ]]; then
    __moriartty_si=1
    __moriartty_prompted=

    __moriartty_precmd() {
        local status=$?
        # The previous command ended. Moriartty ignores this when no command
        # was started (an empty Enter).
        if [[ -n $__moriartty_prompted ]]; then
            printf '\e]133;D;%s\a' "$status"
        fi
        __moriartty_prompted=1
        printf '\e]133;A\a'
        return "$status"
    }

    # PS0 is printed once a command line is read, just before it runs.
    if [[ ${PS0-} != *'\e]133;C\a'* ]]; then
        PS0="${PS0-}"'\e]133;C\a'
    fi

    # First, so it sees the command's exit status.
    if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then
        PROMPT_COMMAND=(__moriartty_precmd "${PROMPT_COMMAND[@]}")
    else
        PROMPT_COMMAND="__moriartty_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
    fi
fi
