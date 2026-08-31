#!/bin/sh
set -eu

fail() {
    printf '%s\n' "flow-install: $1" >&2
    exit 1
}

prefix=
install_executor=1
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || fail 'missing value for --prefix'
            [ -z "$prefix" ] || fail '--prefix may be supplied only once'
            prefix=$2
            shift 2
            ;;
        --no-default-executor)
            [ "$install_executor" -eq 1 ] || fail '--no-default-executor may be supplied only once'
            install_executor=0
            shift
            ;;
        *) fail "unknown argument: $1" ;;
    esac
done

[ -n "$prefix" ] || fail 'usage: install.sh --prefix <absolute-prefix> [--no-default-executor]'
case "$prefix" in
    /*) ;;
    *) fail '--prefix must be absolute' ;;
esac

case "$0" in
    /*) installer=$0 ;;
    *) installer=$PWD/$0 ;;
esac
[ ! -L "$installer" ] || fail 'installer must not be a symbolic link'
bundle=${installer%/*}
[ "$bundle" != "$installer" ] || fail 'installer bundle is unavailable'
bundle_mode=$(/usr/bin/stat -c '%a' -- "$bundle") || fail 'cannot inspect installer bundle mode'
[ $((0$bundle_mode & 0022)) -eq 0 ] || fail 'installer bundle is writable by other users'
bundle_owner=$(/usr/bin/stat -c '%u' -- "$bundle") || fail 'cannot inspect installer bundle owner'
current_owner=$(/usr/bin/id -u) || fail 'cannot inspect installer owner'
[ "$bundle_owner" -eq 0 ] || [ "$bundle_owner" -eq "$current_owner" ] || fail 'untrusted installer bundle owner'

flow_source=$bundle/flow
executor_source=$bundle/flow-executor

validate_source() {
    source_path=$1
    [ -f "$source_path" ] || fail "missing regular bundle artifact: $source_path"
    [ ! -L "$source_path" ] || fail "linked bundle artifact is unsafe: $source_path"
    [ -x "$source_path" ] || fail "bundle artifact is not executable: $source_path"
    source_links=$(/usr/bin/stat -c '%h' -- "$source_path") || fail 'cannot inspect bundle artifact'
    [ "$source_links" -eq 1 ] || fail "hard-linked bundle artifact is unsafe: $source_path"
    source_mode=$(/usr/bin/stat -c '%a' -- "$source_path") || fail 'cannot inspect bundle artifact mode'
    [ $((0$source_mode & 0022)) -eq 0 ] || fail "writable bundle artifact is unsafe: $source_path"
    source_owner=$(/usr/bin/stat -c '%u' -- "$source_path") || fail 'cannot inspect bundle artifact owner'
    current_owner=$(/usr/bin/id -u) || fail 'cannot inspect installer owner'
    [ "$source_owner" -eq 0 ] || [ "$source_owner" -eq "$current_owner" ] || fail "untrusted bundle artifact owner: $source_path"
}

validate_source "$flow_source"
if [ "$install_executor" -eq 1 ]; then
    validate_source "$executor_source"
fi

bin=$prefix/bin
if [ -e "$bin" ] || [ -L "$bin" ]; then
    [ -d "$bin" ] && [ ! -L "$bin" ] || fail 'installation bin path is unsafe'
else
    old_umask=$(umask)
    umask 022
    /bin/mkdir -p -- "$bin" || fail 'cannot create installation bin directory'
    umask "$old_umask"
fi

bin_mode=$(/usr/bin/stat -c '%a' -- "$bin") || fail 'cannot inspect installation bin mode'
[ $((0$bin_mode & 0022)) -eq 0 ] || fail 'installation bin directory is writable by other users'
bin_owner=$(/usr/bin/stat -c '%u' -- "$bin") || fail 'cannot inspect installation bin owner'
current_owner=$(/usr/bin/id -u) || fail 'cannot inspect installer owner'
[ "$bin_owner" -eq "$current_owner" ] || fail 'installation bin directory is not owned by the installer administrator'

flow_target=$bin/flow
executor_target=$bin/flow-executor
[ ! -e "$flow_target" ] && [ ! -L "$flow_target" ] || fail 'existing installation is not upgraded'
[ ! -e "$executor_target" ] && [ ! -L "$executor_target" ] || fail 'existing installation is not upgraded'

flow_stage=$bin/.flow.install.$$
executor_stage=$bin/.flow-executor.install.$$
readiness_config=$bin/.flow-readiness-config.$$
readiness_status_file=$readiness_config/status
published_flow=0
published_executor=0
readiness_config_created=0
readiness_pid=
readiness_pgid=
readiness_group_has_descendant() {
    for readiness_member in $(/usr/bin/pgrep -g "$readiness_pgid" 2>/dev/null || :); do
        [ "$readiness_member" = "$readiness_pid" ] || return 0
    done
    return 1
}
wait_for_readiness_group() {
    wait_attempts=20
    while readiness_group_has_descendant; do
        [ "$wait_attempts" -gt 0 ] || return 1
        /bin/sleep 0.05
        wait_attempts=$((wait_attempts - 1))
    done
}
wait_for_readiness_status() {
    # Six seconds permits the five-second checker timeout plus reporting overhead.
    readiness_attempts=120
    while [ ! -e "$readiness_status_file" ] && [ ! -L "$readiness_status_file" ]; do
        [ "$readiness_attempts" -gt 0 ] || return 1
        /bin/sleep 0.05
        readiness_attempts=$((readiness_attempts - 1))
    done
    [ -f "$readiness_status_file" ] && [ ! -L "$readiness_status_file" ] || return 1
    readiness_status_metadata=$(/usr/bin/stat -c '%u:%h:%s' -- "$readiness_status_file") || return 1
    case "$readiness_status_metadata" in
        "$current_owner:1:2"|"$current_owner:1:3"|"$current_owner:1:4") ;;
        *) return 1 ;;
    esac
    readiness_status=$(/bin/cat -- "$readiness_status_file") || return 1
    case "$readiness_status" in
        ''|*[!0-9]*) return 1 ;;
    esac
    [ "$readiness_status" -le 255 ] || return 1
}
stop_readiness() {
    [ -n "$readiness_pid" ] || return 0
    /bin/kill -TERM -- "-$readiness_pgid" 2>/dev/null || :
    /bin/kill -TERM -- "$readiness_pid" 2>/dev/null || :
    if ! wait_for_readiness_group; then
        /bin/kill -KILL -- "-$readiness_pgid" 2>/dev/null || :
        /bin/kill -KILL -- "$readiness_pid" 2>/dev/null || :
        wait_for_readiness_group || :
    fi
    wait "$readiness_pid" 2>/dev/null || :
    readiness_pid=
    readiness_pgid=
}
cleanup() {
    stop_readiness
    /bin/rm -f -- "$flow_stage" "$executor_stage" || :
    if [ "$readiness_config_created" -eq 1 ]; then
        /bin/rm -rf -- "$readiness_config" || :
    fi
    if [ "$published_executor" -eq 1 ]; then
        /bin/rm -f -- "$executor_target" || :
    fi
    if [ "$published_flow" -eq 1 ]; then
        /bin/rm -f -- "$flow_target" || :
    fi
}
signal_exit() {
    signal_status=$1
    trap '' HUP INT TERM
    exit "$signal_status"
}
trap cleanup EXIT
trap 'signal_exit 129' HUP
trap 'signal_exit 130' INT
trap 'signal_exit 143' TERM

/bin/cp --reflink=never --no-preserve=mode,ownership,timestamps -- "$flow_source" "$flow_stage" \
    || fail 'cannot stage flow'
/bin/chmod 0755 -- "$flow_stage" || fail 'cannot protect staged flow'
if [ "$install_executor" -eq 1 ]; then
    /bin/cp --reflink=never --no-preserve=mode,ownership,timestamps -- "$executor_source" "$executor_stage" \
        || fail 'cannot stage flow-executor'
    /bin/chmod 0755 -- "$executor_stage" || fail 'cannot protect staged flow-executor'
fi

/bin/ln -- "$flow_stage" "$flow_target" || fail 'cannot publish flow'
published_flow=1
/bin/rm -- "$flow_stage" || fail 'cannot finalize flow publication'
if [ "$install_executor" -eq 1 ]; then
    /bin/ln -- "$executor_stage" "$executor_target" || fail 'cannot publish flow-executor'
    published_executor=1
    /bin/rm -- "$executor_stage" || fail 'cannot finalize flow-executor publication'
    /bin/mkdir -m 0700 -- "$readiness_config" || fail 'cannot isolate readiness configuration'
    readiness_config_created=1
    /usr/bin/setsid /bin/sh -c '
        umask 077
        PATH=
        HOME=$1
        XDG_CONFIG_HOME=$1
        export PATH HOME XDG_CONFIG_HOME
        if cd / && "$2" executor check </dev/null; then
            readiness_status=0
        else
            readiness_status=$?
        fi
        printf "%s\\n" "$readiness_status" > "$3.pending" && /bin/mv -f -- "$3.pending" "$3" || :
        while :; do /bin/sleep 3600; done
    ' flow-readiness "$readiness_config" "$flow_target" "$readiness_status_file" &
    readiness_pid=$!
    readiness_pgid=$readiness_pid
    wait_for_readiness_status || fail 'installed Default Executor did not report readiness'
    stop_readiness
    if [ "$readiness_status" -ne 0 ]; then
        fail 'installed Default Executor failed readiness'
    fi
    /bin/rm -- "$readiness_status_file" || fail 'cannot remove readiness status'
    /bin/rmdir -- "$readiness_config" || fail 'cannot remove readiness configuration'
    readiness_config_created=0
fi

published_flow=0
published_executor=0
trap - EXIT HUP INT TERM
printf '%s\n' "installed flow in $bin"
