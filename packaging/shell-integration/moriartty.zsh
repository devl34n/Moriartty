# Moriartty shell integration for zsh.
#
# Marks where each prompt starts and when each command starts and ends, with
# invisible OSC 133 escape sequences. Moriartty uses them to jump between
# prompts (Ctrl+Shift+Up / Ctrl+Shift+Down) and to tell you when a long
# command ends in a background tab. Nothing leaves the terminal.
#
# Add this to the end of ~/.zshrc:
#
#   [[ -r /usr/share/moriartty/shell-integration/moriartty.zsh ]] &&
#       . /usr/share/moriartty/shell-integration/moriartty.zsh

# Interactive shells in Moriartty only, and only once.
if [[ -o interactive && ${TERM_PROGRAM-} == moriartty && -z ${__moriartty_si-} ]]; then
    typeset -g __moriartty_si=1 __moriartty_prompted=

    __moriartty_precmd() {
        local ret=$?
        # The previous command ended. Moriartty ignores this when no command
        # was started.
        if [[ -n $__moriartty_prompted ]]; then
            print -n "\e]133;D;${ret}\a"
        fi
        __moriartty_prompted=1
        print -n '\e]133;A\a'
        return $ret
    }

    __moriartty_preexec() {
        print -n '\e]133;C\a'
    }

    # First, so it sees the command's exit status.
    typeset -ga precmd_functions preexec_functions
    precmd_functions=(__moriartty_precmd ${precmd_functions:#__moriartty_precmd})
    preexec_functions+=(__moriartty_preexec)
fi
