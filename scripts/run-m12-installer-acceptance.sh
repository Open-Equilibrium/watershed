#!/bin/sh
set -eu
unset FLOW_AGENT_HOME XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS

: "${RUNNER_TEMP:?RUNNER_TEMP must name a private temporary directory}"
if [ "${1-}" != --bounded ]; then
  exec node scripts/run-python.mjs scripts/m12_native.py acceptance "$0"
fi
shift
acceptance_root=$(mktemp -d "$RUNNER_TEMP/m12-installer.XXXXXX")
acceptance_root=$(cd "$acceptance_root" && /bin/pwd -P)

bundle="$acceptance_root/m12-install-bundle"
acceptance_bundle="$acceptance_root/m12-acceptance-install-bundle"
acceptance_prefix="$acceptance_root/m12-acceptance-prefix"
standard_prefix="$acceptance_root/m12-standard-prefix"
custom_prefix="$acceptance_root/m12-custom-prefix"
config="$acceptance_root/m12-config"
home="$acceptance_root/m12-home"
agent_home="$acceptance_root/m12-agent-home"
fixture_home="$acceptance_root/m12-fixture-home"
fixture_error="$acceptance_root/m12-fixture-smoke.stderr"
fixture_output="$acceptance_root/m12-fixture-smoke.jsonl"
fixture_workspace="$acceptance_root/m12-fixture-workspace"
productive_workspace="$acceptance_root/m12-productive-workspace"
unavailable_workspace="$acceptance_root/m12-unavailable-workspace"
case "$(/usr/bin/uname -s)" in
  Linux) expected_platform=ubuntu-24.04-x86_64; expected_backend=bubblewrap-seccomp ;;
  Darwin)
    expected_platform=macos-26-aarch64
    expected_backend=seatbelt
    config="$home/Library/Application Support"
    ;;
  *) printf 'native installer acceptance requires Linux or macOS\n' >&2; exit 1 ;;
esac
run_local() {
  /usr/bin/env PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$@"
}
run_in_workspace() {
  workspace=$1
  shift
  run_local /bin/sh -c 'cd "$1" && shift && exec "$@"' \
    flow-workspace "$workspace" "$@"
}
install -d -m 0755 "$bundle" "$acceptance_bundle"
install -d -m 0700 "$config" "$home" "$agent_home" "$fixture_home" "$fixture_workspace" "$productive_workspace" "$unavailable_workspace"
install -m 0755 install/install.sh "$bundle/install.sh"
install -m 0755 target/m12-standard/release/flow "$bundle/flow"
install -m 0755 target/m12-standard/release/flow-executor "$bundle/flow-executor"
install -m 0755 install/install.sh "$acceptance_bundle/install.sh"
install -m 0755 target/m12-acceptance/release/flow "$acceptance_bundle/flow"
install -m 0755 target/m12-standard/release/flow-executor "$acceptance_bundle/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$standard_prefix")
test -x "$standard_prefix/bin/flow"
test -x "$standard_prefix/bin/flow-executor"
run_local "$standard_prefix/bin/flow" executor check </dev/null
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$acceptance_bundle/install.sh" --prefix "$acceptance_prefix")
test -x "$acceptance_prefix/bin/flow"
test -x "$acceptance_prefix/bin/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$custom_prefix" --no-default-executor)
test -x "$custom_prefix/bin/flow"
test ! -e "$custom_prefix/bin/flow-executor"
set +e
unavailable=$(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$custom_prefix/bin/flow" executor check 2>&1)
unavailable_status=$?
set -e
test "$unavailable_status" -eq 65
case "$unavailable" in
  "error: executor_unavailable:"*) ;;
  *) exit 1 ;;
esac
run_in_workspace "$fixture_workspace" /usr/bin/env FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" init --registry-root registry
cp -R flow-agent/fixtures/smoke-flow/registry/. "$fixture_home/registry/"
printf '%s\n' \
  'fixture_profile: stub-model' \
  'stub_model: deterministic' \
  >> "$fixture_home/config.yaml"
run_in_workspace "$fixture_workspace" /usr/bin/env FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" validate smoke-flow
set +e
run_in_workspace "$fixture_workspace" /usr/bin/env FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" run smoke-flow --emit jsonl > "$fixture_output" 2> "$fixture_error"
fixture_status=$?
set -e
if [ "$fixture_status" -ne 0 ]; then
  printf 'fixture run failed with exit %s\n' "$fixture_status" >&2
  cat "$fixture_error" >&2
  exit 1
fi
diff -u flow-agent/fixtures/smoke-flow/expected/smoke-flow.jsonl "$fixture_output"
install -d -m 0700 "$config/flow-agent"
cp -R flow-agent/fixtures/smoke-flow/registry "$agent_home/registry"
printf '%s\n' \
  'model: gpt-m12-install-acceptance' \
  'provider: openai-codex' \
  'model_context_limit: 128000' \
  'output_reserve: 16384' \
  'registry_root: registry' \
  > "$agent_home/config.yaml"
printf '%s\n' \
  '{"openai-codex":{"type":"oauth","access":"ci-inert-access","refresh":"ci-inert-refresh","expires":18446744073709551615,"accountId":"ci-inert-account","isFedramp":false}}' \
  > "$config/flow-agent/credentials.json"
chmod 0600 "$agent_home/config.yaml" "$config/flow-agent/credentials.json"
test ! -e "$config/flow-agent/executor.json"
productive_output=$(
  run_in_workspace "$productive_workspace" /usr/bin/env \
    FLOW_AGENT_HOME="$agent_home" FLOW_AGENT_M12_INSTALL_ACCEPTANCE=1 \
    "$acceptance_prefix/bin/flow" run smoke-flow
)
case "$productive_output" in
  "flow smoke-flow (conversation "*", run "*") completed") ;;
  *)
    printf 'standard installation productive Flow returned an unexpected result\n%s\n' \
      "$productive_output" >&2
    exit 1
    ;;
esac
test ! -e "$config/flow-agent/executor.json"
node scripts/run-python.mjs - "$agent_home" "$expected_platform" "$expected_backend" <<'PY'
import json
import pathlib
import sys

home = pathlib.Path(sys.argv[1])
logs = list(
    (home / "workspaces").glob(
        "workspace-v1-*/sessions/*/runs/*/run-log.jsonl"
    )
)
assert len(logs) == 1, logs
records = [json.loads(line) for line in logs[0].read_text(encoding="utf-8").splitlines()]
provider_intents = [
    record
    for record in records
    if record.get("record_type") == "intent" and record.get("attempt_kind") == "provider"
]
provider_results = [
    record
    for record in records
    if record.get("record_type") == "terminal-result"
    and record.get("attempt_kind") == "provider"
    and record.get("outcome") == "completed"
]
tool_results = [
    record
    for record in records
    if record.get("record_type") == "terminal-result"
    and record.get("attempt_kind") == "tool"
    and record.get("tool_id") == "echo"
]
assert len(provider_intents) == 2, provider_intents
assert len(provider_results) == 2, provider_results
assert len(tool_results) == 1, tool_results
tool = tool_results[0]
assert tool.get("outcome") == "completed", tool
assert tool.get("exit_code") == 0, tool
durable = tool["durable_output"]
assert durable["schema"] == "flow-tool-attempt-output-v1", durable
assert durable["request_hash"].startswith("sha256:"), durable
receipt = durable["enforcement"]
assert receipt["executor"] == "flow-executor", receipt
assert set(receipt) == {"applied_policy_digest", "backend", "backend_version",
                        "executor", "executor_version", "self_protection_active", "platform"}, receipt
assert receipt["self_protection_active"] is True, receipt
assert receipt["platform"] == sys.argv[2], receipt
assert receipt["backend"] == sys.argv[3], receipt
assert receipt["applied_policy_digest"].startswith("sha256:"), receipt
PY
set +e
productive_unavailable=$(run_in_workspace "$unavailable_workspace" /usr/bin/env FLOW_AGENT_HOME="$agent_home" "$custom_prefix/bin/flow" run smoke-flow 2>&1)
productive_unavailable_status=$?
set -e
if [ "$productive_unavailable_status" -ne 65 ]; then
  printf 'productive run without an Executor returned exit %s, expected 65\n%s\n' \
    "$productive_unavailable_status" "$productive_unavailable" >&2
  exit 1
fi
case "$productive_unavailable" in
  "error: executor_unavailable:"*) ;;
  *)
    printf 'productive run without an Executor returned an unexpected diagnostic\n%s\n' \
      "$productive_unavailable" >&2
    exit 1
    ;;
esac
if [ -e "$unavailable_workspace/.flow" ]; then
  printf 'productive Executor preflight mutated the workspace\n' >&2
  exit 1
fi
check_custom_selection() {
  run_local "$custom_prefix/bin/flow" executor configure --path "$bundle/flow-executor"
  test -f "$config/flow-agent/executor.json"
  run_local "$custom_prefix/bin/flow" executor check </dev/null
  ln -s "$custom_prefix/bin/absent-executor" "$custom_prefix/bin/flow-executor"
  if run_local "$custom_prefix/bin/flow" executor check </dev/null; then
    printf 'Custom selection ignored an unsafe installed sibling\n' >&2
    exit 1
  else
    test "$?" -eq 65
  fi
  if run_local "$custom_prefix/bin/flow" executor configure --path "$bundle/flow-executor"; then
    printf 'Custom configuration ignored an unsafe installed sibling\n' >&2
    exit 1
  else
    test "$?" -eq 65
  fi
  test -L "$custom_prefix/bin/flow-executor"
  rm -- "$custom_prefix/bin/flow-executor"
  run_local "$custom_prefix/bin/flow" executor check </dev/null
  run_local "$custom_prefix/bin/flow" executor configure --default
  test ! -e "$config/flow-agent/executor.json"
  if run_local "$custom_prefix/bin/flow" executor check </dev/null; then
    printf 'Default selection accepted its missing required Executor\n' >&2
    exit 1
  else
    test "$?" -eq 65
  fi
  run_local "$standard_prefix/bin/flow" executor check </dev/null
}
check_custom_selection
if [ -n "${M12_COVERAGE_BIN_DIR:-}" ]; then
  coverage_flow="$M12_COVERAGE_BIN_DIR/flow"
  coverage_executor="$M12_COVERAGE_BIN_DIR/flow-executor"
  test -x "$coverage_flow"
  test -x "$coverage_executor"
  coverage_bundle="$acceptance_root/m12-coverage-bundle"
  standard_prefix="$acceptance_root/m12-coverage-standard"
  custom_prefix="$acceptance_root/m12-coverage-custom"
  install -d -m 0755 "$coverage_bundle"
  install -m 0755 install/install.sh "$coverage_bundle/install.sh"
  install -m 0755 "$coverage_flow" "$coverage_bundle/flow"
  install -m 0755 "$coverage_executor" "$coverage_bundle/flow-executor"
  bundle=$coverage_bundle
  # Exercise the actual installer with instrumented binaries, not a post-install swap.
  (cd / && run_local /bin/sh "$bundle/install.sh" --prefix "$standard_prefix")
  (cd / && run_local /bin/sh "$bundle/install.sh" --prefix "$custom_prefix" --no-default-executor)
  run_local "$standard_prefix/bin/flow" executor check </dev/null
  check_custom_selection
fi
