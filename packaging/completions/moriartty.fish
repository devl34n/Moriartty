# fish completion for moriartty

function __moriartty_sessions
    set -l dir (set -q XDG_CONFIG_HOME; and echo $XDG_CONFIG_HOME; or echo $HOME/.config)/moriartty/sessions
    for f in $dir/*.toml
        basename $f .toml
    end
end

complete -c moriartty -o load -x -a '(__moriartty_sessions)' -d 'Use a saved session'
complete -c moriartty -o config -d 'Open the settings window first'
complete -c moriartty -o title -o T -x -d 'Window title'
complete -c moriartty -o geometry -x -d 'Columns x rows, e.g. 120x40'
complete -c moriartty -o dir -l working-directory -x -a '(__fish_complete_directories)' -d 'Start in directory'
complete -c moriartty -o hold -d 'Keep the window open after the program exits'
complete -c moriartty -o ls -d 'Run the shell as a login shell'
complete -c moriartty -o log -r -F -d 'Log all session output to a file'
complete -c moriartty -o e -x -a '(__fish_complete_command)' -d 'Run a command instead of the shell'
complete -c moriartty -o help -d 'Show usage'
complete -c moriartty -o version -d 'Show the version'
