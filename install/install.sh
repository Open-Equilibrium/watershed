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

release_supported() {
    case "$1:$2" in
        Linux:x86_64)
            /usr/bin/awk '
                function exact(value, literal) {
                    return value == literal || value == "\"" literal "\"" ||
                        value == "\047" literal "\047"
                }
                /^ID=/ { ids++; id = substr($0, 4) }
                /^VERSION_ID=/ { versions++; version = substr($0, 12) }
                END { exit !(ids == 1 && versions == 1 &&
                    exact(id, "ubuntu") && exact(version, "24.04")) }
            '
            ;;
        Darwin:arm64)
            /usr/bin/awk '
                { release = release $0 "\n" }
                END {
                    gsub(/^[[:space:]]+|[[:space:]]+$/, "", release)
                    exit (release !~ /^27(\.[0-9]+)+$/)
                }
            '
            ;;
        *) return 1 ;;
    esac
}

validate_acl() {
    [ "$host" = Darwin ] || return 0
    [ -x /usr/bin/osascript ] || fail 'macOS ACL admission requires /usr/bin/osascript with JavaScript and Objective-C support'
    # Query the held object on stdin. Darwin ACLs have at most 128 ordered entries;
    # only earlier matching/everyone denies neutralize untrusted mutation grants.
    /usr/bin/osascript -l JavaScript -e '
ObjC.import("Foundation");
ObjC.bindFunction("acl_get_fd_np", ["void *", ["int", "int"]]);
ObjC.bindFunction("acl_free", ["int", ["void *"]]);
ObjC.bindFunction("acl_valid", ["int", ["void *"]]);
ObjC.bindFunction("acl_get_entry", ["int", ["void *", "int", "void **"]]);
ObjC.bindFunction("acl_get_tag_type", ["int", ["void *", "int *"]]);
ObjC.bindFunction("acl_get_permset_mask_np", ["int", ["void *", "unsigned long long *"]]);
ObjC.bindFunction("acl_get_flagset_np", ["int", ["void *", "void **"]]);
ObjC.bindFunction("acl_get_flag_np", ["int", ["void *", "int"]]);
ObjC.bindFunction("acl_get_qualifier", ["unsigned char *", ["void *"]]);
ObjC.bindFunction("acl_init", ["void *", ["int"]]);
ObjC.bindFunction("acl_set_fd_np", ["int", ["int", "void *", "int"]]);
ObjC.bindFunction("mbr_uuid_to_id", ["int", ["void *", "unsigned int *", "int *"]]);
ObjC.bindFunction("__error", ["int *", []]);

function isNull(pointer) {
    return $.NSValue.valueWithPointer(pointer).isEqualToValue($.NSValue.valueWithPointer(null));
}
function checked(result) {
    if (result !== 0) throw Error("ACL metadata query failed: " + $.__error()[0]);
}
function run(argv) {
    if (argv.length !== 2 || !/^[0-9]+$/.test(argv[0]) ||
        (argv[1] !== "public" && argv[1] !== "created-private")) throw Error("invalid ACL admission arguments");
    const uid = Number(argv[0]);
    // sys/acl.h and sys/kauth.h: data, namespace, attribute and security mutation.
    const mutation = 0x3574;
    const known = 0x3ffe | (1 << 20) | (0xf << 21);
    const everyone = "abcdefabcdefabcdefabcdef0000000c";
    const owner = "abcdefabcdefabcdefabcdef0000000a";
    const nobody = "abcdefabcdefabcdefabcdeffffffffe";
    // A newly created private directory is first admitted as public, then
    // hardened through the same descriptor and checked for an empty ACL.
    for (let pass = 0; pass < 2; pass++) {
        const acl = $.acl_get_fd_np(0, 256);
        const aclError = $.__error()[0];
        if (isNull(acl)) {
            if (aclError === 2) return;
            throw Error("ACL metadata unavailable: " + aclError);
        }
        try {
            checked($.acl_valid(acl));
            const denied = {};
            let everyoneDenied = 0;
            for (let index = 0; index <= 128; index++) {
                const entry = Ref();
                const result = $.acl_get_entry(acl, index === 0 ? 0 : -1, entry);
                if (result === -1 && $.__error()[0] === 22) break;
                checked(result);
                if (index === 128 || isNull(entry[0])) throw Error("invalid ACL inventory");
                if (pass === 1) throw Error("private directory has extended ACL entries");
                const tag = Ref(), mask = Ref(), flags = Ref();
                checked($.acl_get_tag_type(entry[0], tag));
                checked($.acl_get_permset_mask_np(entry[0], mask));
                checked($.acl_get_flagset_np(entry[0], flags));
                let permissions = mask[0];
                if ((tag[0] !== 1 && tag[0] !== 2) || permissions < 0 || permissions > known ||
                    (permissions & ~known) !== 0) throw Error("unsupported ACL metadata");
                const inheritOnly = $.acl_get_flag_np(flags[0], 1 << 8);
                if (inheritOnly === 1) continue;
                if (inheritOnly !== 0) throw Error("ACL flags unavailable");
                // kauth_acl_evaluate expands generic write/all before evaluating grants.
                if (permissions & ((1 << 23) | (1 << 21))) permissions |= mutation & ~(1 << 13);
                permissions &= mutation;
                if (!permissions) continue;
                const qualifier = $.acl_get_qualifier(entry[0]);
                if (isNull(qualifier)) throw Error("ACL principal unavailable");
                try {
                    // acl_get_qualifier owns exactly one 16-byte Darwin UUID.
                    const principal = Array.from({length: 16}, (_,i) =>
                        qualifier[i].toString(16).padStart(2, "0")).join("");
                    if (principal === nobody) continue;
                    if (tag[0] === 2) {
                        if (principal === everyone) everyoneDenied |= permissions;
                        else denied[principal] = (denied[principal] || 0) | permissions;
                        continue;
                    }
                    if (!(permissions & ~(everyoneDenied | (denied[principal] || 0))) || principal === owner) continue;
                    const id = Ref(), type = Ref();
                    if (!principal.startsWith("abcdefabcdefabcdefabcdef") &&
                        $.mbr_uuid_to_id(qualifier, id, type) === 0 && type[0] === 0 &&
                        (id[0] === 0 || id[0] === uid)) continue;
                    throw Error("ACL permits other-user mutation");
                } finally { $.acl_free(qualifier); }
            }
        } finally {
            $.acl_free(acl);
        }
        if (argv[1] === "public" || pass === 1) return;
        const empty = $.acl_init(0);
        if (isNull(empty)) throw Error("cannot allocate empty private ACL");
        try { checked($.acl_set_fd_np(0, empty, 256)); }
        finally { $.acl_free(empty); }
    }
}
' "$current_owner" "${3-public}" < "$1" \
        || fail "unsafe or unavailable macOS ACL: $2; requires the built-in JavaScript/Objective-C bridge"
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
            /dev/fd/[3-9])
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
[ -f "$bundle/bundle-info" ] && [ ! -L "$bundle/bundle-info" ] \
    || fail 'missing regular bundle-info; extract the complete verified download'
{
    IFS= read -r bundle_version && IFS= read -r bundle_platform && ! IFS= read -r extra && [ -z "$extra" ]
} < "$bundle/bundle-info" || fail 'invalid bundle-info'
printf '%s\n' "$bundle_version" | /usr/bin/grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$' \
    || fail 'invalid bundle version'
machine=$(/usr/bin/uname -m) || fail 'cannot identify bundle target architecture'
case "$bundle_platform:$host:$machine" in
    ubuntu-24.04-x86_64:Linux:x86_64)
        release_supported "$host" "$machine" < /etc/os-release \
            || fail 'bundle requires Ubuntu 24.04 x86_64'
        ;;
    macos-27-aarch64:Darwin:arm64)
        release=$(/usr/bin/sw_vers -productVersion) || fail 'cannot identify macOS release'
        printf '%s\n' "$release" | release_supported "$host" "$machine" \
            || fail 'bundle requires macOS 27 ARM64'
        ;;
    *) fail "bundle target $bundle_platform does not match $host $machine" ;;
esac
exec 3<"$bundle" || fail 'cannot open installer bundle'
bundle_fd=$descriptor_root/3
[ -d "$bundle_fd" ] || fail 'installer bundle is not a directory'
bundle_mode=$(metadata '%a' '%Lp' "$bundle_fd") || fail 'cannot inspect installer bundle mode'
[ $((0$bundle_mode & 0022)) -eq 0 ] || fail 'installer bundle is writable by other users'
bundle_owner=$(metadata '%u' '%u' "$bundle_fd") || fail 'cannot inspect installer bundle owner'
current_owner=$(/usr/bin/id -u) || fail 'cannot inspect installer owner'
[ "$bundle_owner" -eq 0 ] || [ "$bundle_owner" -eq "$current_owner" ] || fail 'untrusted installer bundle owner'
validate_acl "$bundle_fd" 'installer bundle'
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
    validate_acl "$source_path" "$source_name"
}

flow_source_name=$bundle/flow
flow_source_entry=$bundle/flow
[ ! -L "$flow_source_entry" ] || fail "linked bundle artifact is unsafe: $flow_source_name"
[ -f "$flow_source_entry" ] || fail "missing regular bundle artifact: $flow_source_name"
exec 4<"$flow_source_entry" || fail "missing regular bundle artifact: $flow_source_name"
flow_source=$descriptor_root/4
validate_source "$flow_source" "$flow_source_name"
if [ "$install_executor" -eq 1 ]; then
    executor_source_name=$bundle/flow-executor
    executor_source_entry=$bundle/flow-executor
    [ ! -L "$executor_source_entry" ] || fail "linked bundle artifact is unsafe: $executor_source_name"
    [ -f "$executor_source_entry" ] || fail "missing regular bundle artifact: $executor_source_name"
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
validate_acl "$bin_fd" 'installation bin directory'

# The working directory anchors publication and rollback on both native hosts;
# Darwin's descriptor filesystem does not support traversing directory entries.
cd "$bin" || fail 'cannot enter installation bin directory'
matches_descriptor . "$bin_fd" || fail 'installation bin path changed during installation'
flow_target=./flow
executor_target=./flow-executor
[ ! -e "$flow_target" ] && [ ! -L "$flow_target" ] || fail 'existing installation is not upgraded'
[ ! -e "$executor_target" ] && [ ! -L "$executor_target" ] || fail 'existing installation is not upgraded'

stage_directory=./.flow.install.$$
stage_created=0
stage_admitted=0
flow_stage=$stage_directory/flow
executor_stage=$stage_directory/flow-executor
readiness_config=./.flow-readiness-config.$$
readiness_status_file=$readiness_config/status
published_flow=0
published_executor=0
installation_committed=0
readiness_config_created=0
readiness_config_admitted=0
readiness_active=0
readiness_control=$stage_directory/readiness-control
readiness_inner_control=$stage_directory/readiness-inner-control
readiness_scanner=/usr/bin/pgrep
# Every signal is issued by a live member of its own admitted group.
readiness_shell='
    set +e
    trap ":" HUP INT TERM PIPE
    role=$1
    readiness_config=$2
    flow_target=$3
    readiness_status_file=$4
    readiness_inner_control=$5
    installer_pgid=$6
    readiness_scanner=$7
    script=$8
    shift 8
    helper_count=$#
    readiness_pid=$$
    readiness_group=$(/bin/ps -o pgid= -p "$$") || exit 1
    # Preserve the literal helper arguments while checking exactly one group.
    readiness_pgid=$(set -- $readiness_group; [ "$#" -eq 1 ] && printf "%s" "$1") || exit 1
    case "$readiness_pgid" in ""|*[!0-9]*) exit 1 ;; esac
    [ "$readiness_pgid" -gt 1 ] && [ "$readiness_pgid" != "$installer_pgid" ] || exit 1
    readiness_group_has_descendant() {
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
            [ -x "$readiness_scanner" ] || return 1
            [ "$wait_attempts" -gt 0 ] || return 1
            /bin/sleep 0.05
            wait_attempts=$((wait_attempts - 1))
        done
    }
    IFS= read -r request || exit 1
    [ "$request" = start ] || exit 1
    if [ "$role" = outer ]; then
        [ "$helper_count" -gt 0 ] || exit 1
        exec 7<>"$readiness_inner_control" || exit 1
        exec 8>"$readiness_inner_control" || exit 1
        "$@" /bin/sh -c "$script" flow-readiness inner \
            "$readiness_config" "$flow_target" "$readiness_status_file" \
            "$readiness_inner_control" "$installer_pgid" "$readiness_scanner" "" \
            <"$readiness_inner_control" 7>&- 8>&- &
        exec 7>&-
        printf "%s\n" start >&8 || :
        IFS= read -r request || :
        exec 7>&- 8>&-
        wait_for_readiness_group && exit 0
    else
        (
            trap - HUP INT TERM PIPE
            umask 077
            PATH=
            HOME=$(cd "$readiness_config" && /bin/pwd -P) || exit 1
            [ "$HOME" -ef "$readiness_config" ] || exit 1
            XDG_CONFIG_HOME=$HOME
            unset FLOW_AGENT_HOME XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS
            export PATH HOME XDG_CONFIG_HOME
            if "$flow_target" executor check </dev/null; then status=0; else status=$?; fi
            printf "%s\n" "$status" > "$readiness_status_file.pending" && \
                /bin/mv -f -- "$readiness_status_file.pending" "$readiness_status_file" || :
        ) </dev/null &
        IFS= read -r request || :
    fi
    /bin/kill -TERM -- "-$readiness_pgid" 2>/dev/null || :
    wait_for_readiness_group && exit 0
    # This last signal also kills the caller; there is no numeric identity reuse.
    exec /bin/kill -KILL -- "-$readiness_pgid" 2>/dev/null
'
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
    [ "$readiness_active" -eq 1 ] || return 0
    exec 7>&- 8>&-
    # Readiness is the sole installer background job, including a fork interrupted
    # before its process number could be assigned. No saved number is signaled.
    wait 2>/dev/null || :
    readiness_active=0
}
cleanup() {
    trap '' HUP INT TERM
    stop_readiness
    if [ "$readiness_config_created" -eq 1 ]; then
        if [ "$readiness_config_admitted" -eq 1 ]; then
            /bin/rm -rf -- "$readiness_config" || :
        else
            /bin/rmdir -- "$readiness_config" || :
        fi
    fi
    if [ "$installation_committed" -eq 0 ] && [ "$stage_admitted" -eq 1 ]; then
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
    if [ "$stage_created" -eq 1 ]; then
        if [ "$stage_admitted" -eq 1 ]; then
            /bin/rm -f -- "$flow_stage" "$executor_stage" "$readiness_control" "$readiness_inner_control" || :
        fi
        /bin/rmdir -- "$stage_directory" || :
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

/bin/mkdir -m 0700 -- "$stage_directory" || fail 'cannot create installation staging directory'
stage_created=1
exec 7<"$stage_directory" || fail 'cannot open installation staging directory'
stage_fd=$descriptor_root/7
matches_descriptor "$stage_directory" "$stage_fd" || fail 'installation staging directory changed'
validate_acl "$stage_fd" 'installation staging directory' created-private
stage_admitted=1
(umask 077; set -C; /bin/cat <&4 > "$flow_stage") \
    || fail 'cannot stage flow'
/bin/chmod 0755 "$flow_stage" || fail 'cannot protect staged flow'
exec 8<"$flow_stage" || fail 'cannot open staged flow'
validate_source "$descriptor_root/8" "$flow_stage"
if [ "$install_executor" -eq 1 ]; then
    (umask 077; set -C; /bin/cat <&5 > "$executor_stage") \
        || fail 'cannot stage flow-executor'
    /bin/chmod 0755 "$executor_stage" || fail 'cannot protect staged flow-executor'
    exec 9<"$executor_stage" || fail 'cannot open staged flow-executor'
    validate_source "$descriptor_root/9" "$executor_stage"
fi
verify_bundle_binding
matches_descriptor "$stage_directory" "$stage_fd" || fail 'installation staging directory changed'
exec 3<&-
exec 4<&-
exec 7<&-
exec 8<&-
if [ "$install_executor" -eq 1 ]; then
    exec 5<&-
    exec 9<&-
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
    exec 7<"$readiness_config" || fail 'cannot open readiness configuration'
    matches_descriptor "$readiness_config" "$descriptor_root/7" || fail 'readiness configuration changed'
    validate_acl "$descriptor_root/7" 'private readiness configuration' created-private
    readiness_metadata=$(metadata '%u:%a' '%u:%Lp' "$descriptor_root/7") || fail 'cannot inspect readiness configuration'
    [ "$readiness_metadata" = "$current_owner:700" ] || fail 'unsafe private readiness configuration'
    readiness_config_admitted=1
    if [ "$current_owner" -eq 0 ]; then
        if [ "$host" = Darwin ]; then
            owner_command=/usr/sbin/chown
        else
            owner_command=/bin/chown
        fi
        "$owner_command" "$readiness_owner:$readiness_group" "$readiness_config" \
            || fail 'cannot assign readiness configuration'
    fi
    matches_descriptor "$readiness_config" "$descriptor_root/7" || fail 'readiness configuration changed'
    exec 7<&-
    installer_group=$(/bin/ps -o pgid= -p "$$") || fail 'cannot inspect installer process group'
    installer_pgid=$(set -- $installer_group; [ "$#" -eq 1 ] && printf '%s' "$1") \
        || fail 'cannot inspect installer process group'
    case "$installer_pgid" in ''|*[!0-9]*) fail 'cannot inspect installer process group' ;; esac
    readiness_role=inner
    /usr/bin/mkfifo -m 0600 "$readiness_control" || fail 'cannot create readiness control'
    if [ "$current_owner" -eq 0 ]; then
        readiness_role=outer
        /usr/bin/mkfifo -m 0600 "$readiness_inner_control" || fail 'cannot create readiness control'
    fi
    exec 7<>"$readiness_control" || fail 'cannot open readiness control'
    exec 8>"$readiness_control" || fail 'cannot open readiness control'
    set -- /bin/sh -c "$readiness_shell" flow-readiness "$readiness_role" \
        "$readiness_config" "$flow_target" "$readiness_status_file" \
        "$readiness_inner_control" "$installer_pgid" "$readiness_scanner" "$readiness_shell" "$@"
    if [ "$host" = Darwin ]; then
        # macOS /bin/sh gives each background job its own group without a terminal.
        set -m
    else
        set -- /usr/bin/setsid "$@"
    fi
    trap ':' PIPE
    readiness_active=1
    # Open the reader while the inherited RDWR anchor still prevents an open hang.
    "$@" <"$readiness_control" 7>&- 8>&- &
    exec 7>&-
    printf '%s\n' start >&8 || :
    wait_for_readiness_status || fail 'installed Default Executor did not report readiness'
    stop_readiness
    /bin/rm -f -- "$readiness_control" "$readiness_inner_control" || fail 'cannot remove readiness control'
    if [ "$readiness_status" -ne 0 ]; then
        fail 'installed Default Executor failed readiness; resolve the reported cause for productive Tools, or rerun with --no-default-executor for authoring and Fixture execution only'
    fi
    /bin/rm -- "$readiness_status_file" || fail 'cannot remove readiness status'
    /bin/rmdir -- "$readiness_config" || fail 'cannot remove readiness configuration'
    readiness_config_created=0
fi

verify_bin_binding
/bin/rmdir -- "$stage_directory" || fail 'cannot remove installation staging directory'
installation_committed=1
trap - EXIT HUP INT TERM PIPE
exec 6<&-
printf '%s\n' "installed flow $bundle_version ($bundle_platform) in $bin"
