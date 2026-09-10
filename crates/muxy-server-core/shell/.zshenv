# Restore the user's startup directory before zsh reads the remaining startup files.
if [[ ${MUXY_ZDOTDIR_SET-} == 1 ]]; then
    export ZDOTDIR=$MUXY_ORIGINAL_ZDOTDIR
else
    unset ZDOTDIR
fi
unset MUXY_ZDOTDIR_SET MUXY_ORIGINAL_ZDOTDIR
[[ -r ${ZDOTDIR-$HOME}/.zshenv ]] && source "${ZDOTDIR-$HOME}/.zshenv"
[[ -o interactive && ${MUXY_SHELL_INTEGRATION-} == 1 ]] && source "$MUXY_SHELL_INTEGRATION_DIR/muxy.zsh"
