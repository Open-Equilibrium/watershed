#!/bin/sh
set -eu

: "${RUNNER_TEMP:?RUNNER_TEMP must name a private temporary directory}"

bundle="$RUNNER_TEMP/m12-install-bundle"
acceptance_bundle="$RUNNER_TEMP/m12-acceptance-install-bundle"
acceptance_prefix="$RUNNER_TEMP/m12-acceptance-prefix"
standard_prefix="$RUNNER_TEMP/m12-standard-prefix"
custom_prefix="$RUNNER_TEMP/m12-custom-prefix"
config="$RUNNER_TEMP/m12-config"
home="$RUNNER_TEMP/m12-home"
agent_home="$RUNNER_TEMP/m12-agent-home"
fixture_home="$RUNNER_TEMP/m12-fixture-home"
fixture_error="$RUNNER_TEMP/m12-fixture-smoke.stderr"
fixture_output="$RUNNER_TEMP/m12-fixture-smoke.jsonl"
fixture_workspace="$RUNNER_TEMP/m12-fixture-workspace"
productive_workspace="$RUNNER_TEMP/m12-productive-workspace"
unavailable_workspace="$RUNNER_TEMP/m12-unavailable-workspace"
install -d -m 0755 "$bundle" "$acceptance_bundle"
install -d -m 0700 "$config" "$home" "$agent_home" "$fixture_workspace" "$productive_workspace" "$unavailable_workspace"
install -m 0755 install/install.sh "$bundle/install.sh"
install -m 0755 target/m12-standard/release/flow "$bundle/flow"
install -m 0755 target/x86_64-unknown-linux-musl/release/flow-executor "$bundle/flow-executor"
install -m 0755 install/install.sh "$acceptance_bundle/install.sh"
install -m 0755 target/release/flow "$acceptance_bundle/flow"
install -m 0755 target/x86_64-unknown-linux-musl/release/flow-executor "$acceptance_bundle/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$standard_prefix")
test -x "$standard_prefix/bin/flow"
test -x "$standard_prefix/bin/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$standard_prefix/bin/flow" executor check </dev/null)
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
(cd "$fixture_workspace" && PATH= HOME="$home" XDG_CONFIG_HOME="$config" FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" init --registry-root registry)
cp -R flow-agent/fixtures/smoke-flow/registry/. "$fixture_home/registry/"
printf '%s\n' \
  'fixture_profile: stub-model' \
  'stub_model: deterministic' \
  >> "$fixture_home/config.yaml"
(cd "$fixture_workspace" && PATH= HOME="$home" XDG_CONFIG_HOME="$config" FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" validate smoke-flow)
set +e
(cd "$fixture_workspace" && PATH= HOME="$home" XDG_CONFIG_HOME="$config" FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" run smoke-flow --emit jsonl > "$fixture_output" 2> "$fixture_error")
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
  cd "$productive_workspace" &&
    PATH= HOME="$home" XDG_CONFIG_HOME="$config" FLOW_AGENT_HOME="$agent_home" \
      FLOW_AGENT_M12_INSTALL_ACCEPTANCE=1 \
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
/usr/bin/python3 - "$agent_home" <<'PY'
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
assert receipt["isolation_active"] is True, receipt
assert receipt["platform"] == "ubuntu-24.04-x86_64", receipt
assert receipt["runtime_profile"] == "exact", receipt
PY
set +e
productive_unavailable=$(cd "$unavailable_workspace" && PATH= HOME="$home" XDG_CONFIG_HOME="$config" FLOW_AGENT_HOME="$agent_home" "$custom_prefix/bin/flow" run smoke-flow 2>&1)
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
executor="$bundle/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$custom_prefix/bin/flow" executor configure --path "$executor")
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$custom_prefix/bin/flow" executor check </dev/null)
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$standard_prefix/bin/flow" executor configure --default)
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$standard_prefix/bin/flow" executor check </dev/null)
