#!/bin/sh
set -eu

fail() {
    printf '%s\n' "flow-install: $1" >&2
    exit 1
}

usage() {
    printf '%s\n' \
        'Usage: install.sh --prefix <absolute-prefix> [--no-default-executor]' \
        '' \
        'Install Flow Agent on Linux or macOS from sibling bundle artifacts.' \
        '' \
        'Options:' \
        '  --prefix <absolute-prefix>  Install into <absolute-prefix>/bin.' \
        '  --no-default-executor       Install flow without the bundled Default Executor.' \
        '  -h, --help                  Show this help.'
}

prefix=
install_executor=1
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
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

host=$(/usr/bin/uname -s) || fail 'cannot identify installation host'
case "$host" in
    Linux) descriptor_root=/proc/self/fd ;;
    Darwin) descriptor_root=/dev/fd ;;
    *) fail 'installation requires Linux or macOS' ;;
esac
metadata() {
    if [ "$host" = Darwin ]; then
        case "$3" in
            /dev/fd/[3-6])
                # Pathname stat sees the descriptor device, not the held object.
                # BSD stat without a file operand uses fstat on standard input.
                /usr/bin/stat -f "$2" < "$3"
                ;;
            *) /usr/bin/stat -L -f "$2" "$3" ;;
        esac
    else
        /usr/bin/stat -L -c "$1" -- "$3"
    fi
}

matches_descriptor() {
    # Shell -ef compares Darwin's descriptor device instead of its held object.
    path_identity=$(metadata '%d:%i' '%d:%i' "$1") || return 1
    descriptor_identity=$(metadata '%d:%i' '%d:%i' "$2") || return 1
    [ "$path_identity" = "$descriptor_identity" ]
}

case "$0" in
    /*) installer=$0 ;;
    *) installer=$PWD/$0 ;;
esac
[ ! -L "$installer" ] || fail 'installer must not be a symbolic link'
bundle=${installer%/*}
[ "$bundle" != "$installer" ] || fail 'installer bundle is unavailable'
exec 3<"$bundle" || fail 'cannot open installer bundle'
bundle_fd=$descriptor_root/3
[ -d "$bundle_fd" ] || fail 'installer bundle is not a directory'
bundle_mode=$(metadata '%a' '%Lp' "$bundle_fd") || fail 'cannot inspect installer bundle mode'
[ $((0$bundle_mode & 0022)) -eq 0 ] || fail 'installer bundle is writable by other users'
bundle_owner=$(metadata '%u' '%u' "$bundle_fd") || fail 'cannot inspect installer bundle owner'
current_owner=$(/usr/bin/id -u) || fail 'cannot inspect installer owner'
[ "$bundle_owner" -eq 0 ] || [ "$bundle_owner" -eq "$current_owner" ] || fail 'untrusted installer bundle owner'
readiness_owner=$current_owner
if [ "$install_executor" -eq 1 ] && [ "$current_owner" -eq 0 ]; then
    [ -n "${SUDO_USER-}" ] || fail 'root installation requires SUDO_USER for unprivileged readiness'
    readiness_owner=$(/usr/bin/id -u -- "$SUDO_USER") || fail 'cannot inspect readiness user'
    readiness_group=$(/usr/bin/id -g -- "$SUDO_USER") || fail 'cannot inspect readiness user group'
    [ "$readiness_owner" -ne 0 ] || fail 'root is not a valid readiness user'
    if [ "$host" = Darwin ]; then
        set -- /usr/bin/sudo -n -u "$SUDO_USER" --
    else
        set -- /usr/sbin/runuser --user "$SUDO_USER" --
    fi
fi

validate_source() {
    source_path=$1
    source_name=$2
    [ -f "$source_path" ] || fail "missing regular bundle artifact: $source_name"
    [ ! -L "$source_name" ] && matches_descriptor "$source_name" "$source_path" \
        || fail "bundle artifact changed during installation: $source_name"
    # Darwin's descriptor device does not expose the held file's execute access.
    [ -x "$source_name" ] || fail "bundle artifact is not executable: $source_name"
    source_links=$(metadata '%h' '%l' "$source_path") || fail 'cannot inspect bundle artifact'
    [ "$source_links" -eq 1 ] || fail "hard-linked bundle artifact is unsafe: $source_name"
    source_mode=$(metadata '%a' '%Lp' "$source_path") || fail 'cannot inspect bundle artifact mode'
    [ $((0$source_mode & 0022)) -eq 0 ] || fail "writable bundle artifact is unsafe: $source_name"
    source_owner=$(metadata '%u' '%u' "$source_path") || fail 'cannot inspect bundle artifact owner'
    [ "$source_owner" -eq 0 ] || [ "$source_owner" -eq "$current_owner" ] || fail "untrusted bundle artifact owner: $source_name"
}

flow_source_name=$bundle/flow
flow_source_entry=$bundle/flow
[ ! -L "$flow_source_entry" ] || fail "linked bundle artifact is unsafe: $flow_source_name"
exec 4<"$flow_source_entry" || fail "missing regular bundle artifact: $flow_source_name"
flow_source=$descriptor_root/4
validate_source "$flow_source" "$flow_source_name"
if [ "$install_executor" -eq 1 ]; then
    executor_source_name=$bundle/flow-executor
    executor_source_entry=$bundle/flow-executor
    [ ! -L "$executor_source_entry" ] || fail "linked bundle artifact is unsafe: $executor_source_name"
    exec 5<"$executor_source_entry" || fail "missing regular bundle artifact: $executor_source_name"
    executor_source=$descriptor_root/5
    validate_source "$executor_source" "$executor_source_name"
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

exec 6<"$bin" || fail 'cannot open installation bin directory'
bin_fd=$descriptor_root/6
[ -d "$bin_fd" ] || fail 'installation bin path is unsafe'
bin_mode=$(metadata '%a' '%Lp' "$bin_fd") || fail 'cannot inspect installation bin mode'
[ $((0$bin_mode & 0022)) -eq 0 ] || fail 'installation bin directory is writable by other users'
bin_owner=$(metadata '%u' '%u' "$bin_fd") || fail 'cannot inspect installation bin owner'
[ "$bin_owner" -eq "$current_owner" ] || fail 'installation bin directory is not owned by the installer administrator'

# The working directory anchors publication and rollback on both native hosts;
# Darwin's descriptor filesystem does not support traversing directory entries.
cd "$bin" || fail 'cannot enter installation bin directory'
matches_descriptor . "$bin_fd" || fail 'installation bin path changed during installation'
flow_target=./flow
executor_target=./flow-executor
[ ! -e "$flow_target" ] && [ ! -L "$flow_target" ] || fail 'existing installation is not upgraded'
[ ! -e "$executor_target" ] && [ ! -L "$executor_target" ] || fail 'existing installation is not upgraded'

flow_stage=./.flow.install.$$
executor_stage=./.flow-executor.install.$$
readiness_config=./.flow-readiness-config.$$
readiness_status_file=$readiness_config/status
published_flow=0
published_executor=0
installation_committed=0
readiness_config_created=0
readiness_pid=
readiness_pgid=
readiness_scanner=/usr/bin/pgrep
readiness_group_has_descendant() {
    # Darwin's EXIT-trap command substitution can report a missing command as 1.
    [ -x "$readiness_scanner" ] || return 0
    if readiness_members=$("$readiness_scanner" -g "$readiness_pgid" 2>/dev/null); then
        for readiness_member in $readiness_members; do
            [ "$readiness_member" = "$readiness_pid" ] || return 0
        done
        return 1
    else
        readiness_scan_status=$?
        [ "$readiness_scan_status" -eq 1 ] && return 1
        return 0
    fi
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
    readiness_status_metadata=$(metadata '%u:%h:%s' '%u:%l:%z' "$readiness_status_file") || return 1
    case "$readiness_status_metadata" in
        "$readiness_owner:1:2"|"$readiness_owner:1:3"|"$readiness_owner:1:4") ;;
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
    trap '' HUP INT TERM
    stop_readiness
    if [ "$readiness_config_created" -eq 1 ]; then
        /bin/rm -rf -- "$readiness_config" || :
    fi
    if [ "$installation_committed" -eq 0 ]; then
        if [ "$published_executor" -eq 1 ] || {
            [ -e "$executor_stage" ] && [ "$executor_stage" -ef "$executor_target" ]
        }; then
            /bin/rm -f -- "$executor_target" || :
        fi
        if [ "$published_flow" -eq 1 ] || {
            [ -e "$flow_stage" ] && [ "$flow_stage" -ef "$flow_target" ]
        }; then
            /bin/rm -f -- "$flow_target" || :
        fi
    fi
    /bin/rm -f -- "$flow_stage" "$executor_stage" || :
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

verify_bundle_binding() {
    matches_descriptor "$bundle" "$bundle_fd" || fail 'installer bundle path changed during installation'
    matches_descriptor "$flow_source_entry" "$flow_source" || fail 'flow bundle artifact changed during installation'
    if [ "$install_executor" -eq 1 ]; then
        matches_descriptor "$executor_source_entry" "$executor_source" \
            || fail 'flow-executor bundle artifact changed during installation'
    fi
}
verify_bin_binding() {
    [ ! -L "$bin" ] && matches_descriptor "$bin" "$bin_fd" \
        || fail 'installation bin path changed during installation'
}

verify_bundle_binding
verify_bin_binding

(umask 077; set -C; /bin/cat <&4 > "$flow_stage") \
    || fail 'cannot stage flow'
/bin/chmod 0755 "$flow_stage" || fail 'cannot protect staged flow'
if [ "$install_executor" -eq 1 ]; then
    (umask 077; set -C; /bin/cat <&5 > "$executor_stage") \
        || fail 'cannot stage flow-executor'
    /bin/chmod 0755 "$executor_stage" || fail 'cannot protect staged flow-executor'
fi
verify_bundle_binding
exec 3<&-
exec 4<&-
if [ "$install_executor" -eq 1 ]; then
    exec 5<&-
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
    if [ "$current_owner" -eq 0 ]; then
        if [ "$host" = Darwin ]; then
            owner_command=/usr/sbin/chown
        else
            owner_command=/bin/chown
        fi
        "$owner_command" "$readiness_owner:$readiness_group" "$readiness_config" \
            || fail 'cannot assign readiness configuration'
    fi
    if [ "$host" = Darwin ]; then
        # macOS /bin/sh supports job control without a terminal: each background
        # job gets its own process group. No external session helper is required.
        set -m
    else
        set -- /usr/bin/setsid "$@"
    fi
    "$@" /bin/sh -c '
        umask 077
        PATH=
        HOME=$(cd "$1" && /bin/pwd -P) || exit 1
        [ "$HOME" -ef "$1" ] || exit 1
        XDG_CONFIG_HOME=$HOME
        unset FLOW_AGENT_HOME
        unset XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS
        export PATH HOME XDG_CONFIG_HOME
        if "$2" executor check </dev/null; then
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
        fail 'installed Default Executor failed readiness; resolve the reported cause for productive Tools, or rerun with --no-default-executor for authoring and Fixture execution only'
    fi
    /bin/rm -- "$readiness_status_file" || fail 'cannot remove readiness status'
    /bin/rmdir -- "$readiness_config" || fail 'cannot remove readiness configuration'
    readiness_config_created=0
fi

verify_bin_binding
installation_committed=1
trap - EXIT HUP INT TERM
exec 6<&-
printf '%s\n' "installed flow in $bin"
