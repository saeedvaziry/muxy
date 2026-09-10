status is-interactive; or return
if test "$MUXY_SHELL_INTEGRATION" != 1; or set -q __muxy_installed
    return
end
set -g __muxy_installed 1

function __muxy_directory --on-event fish_prompt
    printf '\e]7;file://%s\a' (string escape --style=url -- $PWD | string replace -a '%2F' '/')
end

# Fish 4 emits its own prompt marks. Do not wrap or duplicate them.
if status test-feature mark-prompt 2>/dev/null
    return
end

function __muxy_preexec --on-event fish_preexec
    printf '\e]133;C\a'
end

function __muxy_postexec --on-event fish_postexec
    set -l result $status
    printf '\e]133;D;%d\a' $result
end

function __muxy_status
    return $argv[1]
end

# Wrap after the user's configuration has defined fish_prompt.
function __muxy_install --on-event fish_prompt
    functions --copy fish_prompt __muxy_user_prompt
    function fish_prompt
        set -l result $status
        printf '\e]133;A\a'
        __muxy_status $result
        __muxy_user_prompt
        printf '\e]133;B\a'
    end
    functions --erase __muxy_install
end
