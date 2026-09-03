#!/bin/sh
set -eu

: "${RUNNER_TEMP:?RUNNER_TEMP must name a private temporary directory}"
: "${M12_INSTALL_BUNDLE:?M12_INSTALL_BUNDLE must name the prepared install bundle}"
: "${M12_STANDARD_PREFIX:?M12_STANDARD_PREFIX must name the standard installation}"
: "${M12_CONFIG:?M12_CONFIG must name the acceptance configuration directory}"
: "${M12_HOME:?M12_HOME must name the acceptance home directory}"

negative_prefix="$RUNNER_TEMP/m12-negative-prefix"
negative_root="$RUNNER_TEMP/m12-readiness-negatives"
user_runtime=/run/user/10001

prepare_fault_harness() {
  install -d -m 0755 "$negative_root" /opt/watershed-m12-fault
  install -d -m 0755 "$negative_root/current"
  : > "$negative_root/systemd-run-real"
  # Flow clears the Executor environment, so the wrapper uses this fixed private mount.
  /bin/cat > "$negative_root/systemd-run" <<'WRAPPER'
#!/bin/sh
set -eu
fault_root=/opt/watershed-m12-fault
fault_dir=$fault_root/current
test "$#" -eq 8
test "$1" = --user
test "$2" = --scope
test "$3" = --quiet
test "$4" = --collect
test "$5" = --property=Delegate=pids
test "$6" = --
test "$8" = --capacity-self-test
printf 'scope\n' >> "$fault_dir/systemd-run.calls"
# Preserve the exact production image while the mount namespace alters its host interface.
/bin/cp -- "$7" "$fault_dir/scoped-executor"
/bin/chmod 0755 "$fault_dir/scoped-executor"
exec "$fault_root/systemd-run-real" \
  --user --scope --quiet --collect \
  --property=Delegate=pids -- \
  /usr/bin/bwrap --bind / / -- \
  "$fault_root/barrier" "$fault_dir/scoped-executor" "$8"
WRAPPER
  /bin/cat > "$negative_root/barrier" <<'BARRIER'
#!/bin/sh
set -eu
fault_dir=/opt/watershed-m12-fault/current
/bin/mount --make-rprivate /
IFS=: read -r _ _ cgroup < /proc/self/cgroup
printf '%s %s\n' "$$" "$cgroup" > "$fault_dir/state.pending"
/bin/mv -- "$fault_dir/state.pending" "$fault_dir/state"
IFS= read -r _ < "$fault_dir/release"
LLVM_PROFILE_FILE="$fault_dir/scoped-%p-%m.profraw" \
  exec "$@" 2> "$fault_dir/executor.stderr"
BARRIER
  chmod 0755 "$negative_root/systemd-run-real" "$negative_root/systemd-run" "$negative_root/barrier"
}

run_negative() {
  fault=$1
  expected=$2
  mode=${3-check}
  fault_dir="$negative_root/$fault-$mode"
  install -d -m 0700 -o watershed -g watershed "$fault_dir"
  /usr/bin/mkfifo "$fault_dir/release"
  chown watershed:watershed "$fault_dir/release"
  : > "$fault_dir/empty"
  printf 'memory\n' > "$fault_dir/no-pids"
  chmod 0444 "$fault_dir/empty" "$fault_dir/no-pids"
  M12_FAULT_ROOT="$negative_root" M12_FAULT_DIR="$fault_dir" \
    M12_FAULT_KIND="$fault" M12_FAULT_EXPECTED="$expected" \
    M12_FAULT_MODE="$mode" M12_FAULT_FLOW="$M12_STANDARD_PREFIX/bin/flow" \
    M12_FAULT_INSTALLER="$M12_INSTALL_BUNDLE/install.sh" \
    M12_FAULT_PREFIX="$negative_prefix" M12_FAULT_HOME="$M12_HOME" \
    M12_FAULT_CONFIG="$M12_CONFIG" \
    /usr/bin/unshare --mount /bin/sh -ec '
      command_pid=
      scoped_pid=
      tracer_pid=
      command_status=not-observed
      call_count=not-observed
      phase=mount-harness
      cleanup_fault() {
        cleanup_status=$?
        trap - EXIT
        set +e
        if [ -s "$M12_FAULT_DIR/state" ]; then
          set -- $(/bin/cat "$M12_FAULT_DIR/state")
          /bin/kill -TERM "$1" 2>/dev/null || :
        fi
        [ -z "$tracer_pid" ] || /bin/kill -TERM "$tracer_pid" 2>/dev/null || :
        [ -z "$command_pid" ] || /bin/kill -TERM "$command_pid" 2>/dev/null || :
        if [ "$cleanup_status" -ne 0 ]; then
          printf "readiness negative %s (%s) failed during %s; command status=%s; calls=%s\n" \
            "$M12_FAULT_KIND" "$M12_FAULT_MODE" "$phase" "$command_status" "$call_count" >&2
          for evidence in state stderr executor.stderr strace.stderr; do
            if [ -e "$M12_FAULT_DIR/$evidence" ]; then
              printf "%s:\n" "$evidence" >&2
              /usr/bin/head -c 4096 "$M12_FAULT_DIR/$evidence" >&2
              printf '\n' >&2
            fi
          done
        fi
        exit "$cleanup_status"
      }
      trap cleanup_fault EXIT
      /bin/mount --make-rprivate /
      /bin/mount --bind "$M12_FAULT_ROOT" /opt/watershed-m12-fault
      /bin/mount --bind "$M12_FAULT_DIR" /opt/watershed-m12-fault/current
      /bin/mount --bind /usr/bin/systemd-run /opt/watershed-m12-fault/systemd-run-real
      /bin/mount --bind /opt/watershed-m12-fault/systemd-run /usr/bin/systemd-run
      phase=launch-command
      if [ "$M12_FAULT_MODE" = install ]; then
        (
          cd /
          exec /usr/bin/env PATH= HOME="$M12_FAULT_HOME" \
            XDG_CONFIG_HOME="$M12_FAULT_CONFIG" SUDO_USER=watershed \
            /bin/sh "$M12_FAULT_INSTALLER" --prefix "$M12_FAULT_PREFIX"
        ) > "$M12_FAULT_DIR/stdout" 2> "$M12_FAULT_DIR/stderr" &
      else
        /usr/bin/setpriv --reuid=10001 --regid=10001 --init-groups \
          /usr/bin/env PATH= HOME="$M12_FAULT_HOME" \
            XDG_CONFIG_HOME="$M12_FAULT_CONFIG" \
            "$M12_FAULT_FLOW" executor check \
            > "$M12_FAULT_DIR/stdout" 2> "$M12_FAULT_DIR/stderr" &
      fi
      command_pid=$!
      phase=await-supervisor
      attempts=200
      while [ ! -s "$M12_FAULT_DIR/state" ]; do
        if ! /bin/kill -0 "$command_pid" 2>/dev/null; then
          wait "$command_pid" || :
          /bin/cat "$M12_FAULT_DIR/stderr" >&2
          exit 1
        fi
        [ "$attempts" -gt 0 ] || exit 1
        /bin/sleep 0.01
        attempts=$((attempts - 1))
      done
      set -- $(/bin/cat "$M12_FAULT_DIR/state")
      scoped_pid=$1
      cgroup=$2
      scope=${cgroup%/supervisor}
      phase=assert-private-mount-namespace
      test "$(/usr/bin/readlink /proc/self/ns/mnt)" != \
        "$(/usr/bin/readlink "/proc/$scoped_pid/ns/mnt")"
      phase=inject-fault
      case "$M12_FAULT_KIND" in
        missing-cgroup-v2)
          /usr/bin/nsenter --mount="/proc/$scoped_pid/ns/mnt" -- \
            /bin/mount -t tmpfs -o mode=0755 none /sys/fs/cgroup
          ;;
        missing-pids-controller)
          /usr/bin/nsenter --mount="/proc/$scoped_pid/ns/mnt" -- \
            /bin/mount --bind "$M12_FAULT_DIR/no-pids" \
            "/sys/fs/cgroup$scope/cgroup.controllers"
          ;;
        missing-delegation)
          /usr/bin/nsenter --mount="/proc/$scoped_pid/ns/mnt" -- \
            /bin/mount --bind "$M12_FAULT_DIR/empty" \
            "/sys/fs/cgroup$scope/cgroup.subtree_control"
          ;;
        missing-capacity-events|missing-cleanup-evidence)
          # Pause after the Tool cgroup is created and before its controls are read.
          /usr/bin/strace -e trace=write \
            -e inject=write:delay_enter=2s:when=2 \
            -o "$M12_FAULT_DIR/strace" -p "$scoped_pid" \
            2> "$M12_FAULT_DIR/strace.stderr" &
          tracer_pid=$!
          attempts=200
          while ! /bin/grep -q attached "$M12_FAULT_DIR/strace.stderr"; do
            /bin/kill -0 "$tracer_pid" 2>/dev/null || exit 1
            [ "$attempts" -gt 0 ] || exit 1
            /bin/sleep 0.01
            attempts=$((attempts - 1))
          done
          printf 'release\n' > "$M12_FAULT_DIR/release"
          attempts=200
          while [ ! -d "/sys/fs/cgroup$scope/tool" ]; do
            /bin/kill -0 "$scoped_pid" 2>/dev/null || exit 1
            [ "$attempts" -gt 0 ] || exit 1
            /bin/sleep 0.01
            attempts=$((attempts - 1))
          done
          if [ "$M12_FAULT_KIND" = missing-capacity-events ]; then
            control=pids.events
          else
            control=cgroup.events
          fi
          /usr/bin/nsenter --mount="/proc/$scoped_pid/ns/mnt" -- \
            /bin/mount --bind "$M12_FAULT_DIR/empty" \
            "/sys/fs/cgroup$scope/tool/$control"
          ;;
        *) exit 1 ;;
      esac
      case "$M12_FAULT_KIND" in
        missing-capacity-events|missing-cleanup-evidence) ;;
        *) printf 'release\n' > "$M12_FAULT_DIR/release" ;;
      esac
      phase=await-command
      set +e
      wait "$command_pid"
      status=$?
      command_status=$status
      set -e
      command_pid=
      if [ -n "$tracer_pid" ]; then
        wait "$tracer_pid" || :
        tracer_pid=
      fi
      phase=assert-status
      if [ "$M12_FAULT_MODE" = check ]; then
        test "$status" -eq 65
      else
        test "$status" -ne 0
        phase=assert-rollback
        test ! -e "$M12_FAULT_PREFIX/bin/flow"
        test ! -e "$M12_FAULT_PREFIX/bin/flow-executor"
      fi
      phase=assert-empty-stdout
      test ! -s "$M12_FAULT_DIR/stdout"
      phase=assert-public-diagnostic
      /bin/grep -Fq "executor_unavailable:" "$M12_FAULT_DIR/stderr"
      phase=assert-private-diagnostic
      /bin/grep -Fq "$M12_FAULT_EXPECTED" "$M12_FAULT_DIR/executor.stderr"
      phase=assert-call-count
      call_count=$(/usr/bin/wc -l < "$M12_FAULT_DIR/systemd-run.calls")
      test "$call_count" -eq 1
      scoped_pid=
      trap - EXIT
    '
  test ! -e "$M12_CONFIG/flow-agent/executor.json"
}

prepare_fault_harness
run_negative missing-cgroup-v2 "failed to create Executor supervisor cgroup"
run_negative missing-pids-controller "lacks the pids controller"
run_negative missing-delegation "failed to write cgroup.subtree_control"
run_negative missing-capacity-events "pids.events omits max"
run_negative missing-cleanup-evidence "cgroup.events omits populated"
run_negative missing-delegation "failed to write cgroup.subtree_control" install

restart_user_manager() {
  systemctl start user-runtime-dir@10001.service user@10001.service
}
trap restart_user_manager EXIT
systemctl stop user@10001.service user-runtime-dir@10001.service
test ! -S "$user_runtime/bus"
set +e
missing_manager=$(
  /usr/bin/setpriv --reuid=10001 --regid=10001 --init-groups \
    /usr/bin/env PATH= HOME="$M12_HOME" XDG_CONFIG_HOME="$M12_CONFIG" \
      "$M12_STANDARD_PREFIX/bin/flow" executor check 2>&1
)
missing_manager_status=$?
set -e
test "$missing_manager_status" -eq 65
case "$missing_manager" in
  "error: executor_unavailable:"*) ;;
  *) exit 1 ;;
esac
set +e
missing_manager_install=$(
  cd / && PATH= HOME="$M12_HOME" XDG_CONFIG_HOME="$M12_CONFIG" SUDO_USER=watershed \
    /bin/sh "$M12_INSTALL_BUNDLE/install.sh" --prefix "$negative_prefix" 2>&1
)
missing_manager_install_status=$?
set -e
test "$missing_manager_install_status" -ne 0
case "$missing_manager_install" in
  *"readiness user has no active systemd user manager"*) ;;
  *) exit 1 ;;
esac
test ! -e "$negative_prefix/bin/flow"
test ! -e "$negative_prefix/bin/flow-executor"
test ! -e "$M12_CONFIG/flow-agent/executor.json"
restart_user_manager
trap - EXIT
systemctl is-active --quiet user@10001.service
test -S "$user_runtime/bus"
