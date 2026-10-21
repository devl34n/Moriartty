# bash completion for moriartty
_moriartty() {
    local cur prev i
    cur=${COMP_WORDS[COMP_CWORD]}
    prev=${COMP_WORDS[COMP_CWORD-1]}

    # After -e, everything is the command and its arguments.
    for (( i = 1; i < COMP_CWORD; i++ )); do
        if [[ ${COMP_WORDS[i]} == -e ]]; then
            if (( COMP_CWORD == i + 1 )); then
                COMPREPLY=( $(compgen -c -- "$cur") )
            else
                COMPREPLY=( $(compgen -f -- "$cur") )
            fi
            return
        fi
    done

    case $prev in
        -load)
            local dir=${XDG_CONFIG_HOME:-$HOME/.config}/moriartty/sessions f names=()
            for f in "$dir"/*.toml; do
                [[ -e $f ]] || continue
                f=${f##*/}
                names+=( "${f%.toml}" )
            done
            local IFS=$'\n'
            COMPREPLY=( $(compgen -W "${names[*]}" -- "$cur") )
            return
            ;;
        -dir|--working-directory)
            COMPREPLY=( $(compgen -d -- "$cur") )
            return
            ;;
        -log)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return
            ;;
        -title|-T|-geometry)
            return
            ;;
    esac

    COMPREPLY=( $(compgen -W "-load -config -title -T -geometry -dir --working-directory -hold -ls -log -e -help -version" -- "$cur") )
}
complete -F _moriartty moriartty
