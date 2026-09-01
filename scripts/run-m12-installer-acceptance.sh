#!/bin/sh
set -eu

: "${RUNNER_TEMP:?RUNNER_TEMP must name a private temporary directory}"

bundle="$RUNNER_TEMP/m12-install-bundle"
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
install -d -m 0755 "$bundle"
install -d -m 0700 "$config" "$home" "$agent_home" "$fixture_workspace" "$productive_workspace"
install -m 0755 install/install.sh "$bundle/install.sh"
install -m 0755 target/release/flow "$bundle/flow"
install -m 0755 target/x86_64-unknown-linux-musl/release/flow-executor "$bundle/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$standard_prefix")
test -x "$standard_prefix/bin/flow"
test -x "$standard_prefix/bin/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$standard_prefix/bin/flow" executor check </dev/null)
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
  'model: gpt-ci-inert' \
  'provider: openai-codex' \
  'model_context_limit: 128000' \
  'output_reserve: 16384' \
  'registry_root: registry' \
  > "$agent_home/config.yaml"
printf '%s\n' \
  '{"openai-codex":{"type":"oauth","access":"ci-inert-access","refresh":"ci-inert-refresh","expires":18446744073709551615,"accountId":"ci-inert-account","isFedramp":false}}' \
  > "$config/flow-agent/credentials.json"
chmod 0600 "$agent_home/config.yaml" "$config/flow-agent/credentials.json"
set +e
productive_unavailable=$(cd "$productive_workspace" && PATH= HOME="$home" XDG_CONFIG_HOME="$config" FLOW_AGENT_HOME="$agent_home" "$custom_prefix/bin/flow" run smoke-flow 2>&1)
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
if [ -e "$productive_workspace/.flow" ]; then
  printf 'productive Executor preflight mutated the workspace\n' >&2
  exit 1
fi
executor="$PWD/target/x86_64-unknown-linux-musl/release/flow-executor"
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$custom_prefix/bin/flow" executor configure --path "$executor")
(cd / && PATH= HOME="$home" XDG_CONFIG_HOME="$config" "$custom_prefix/bin/flow" executor check </dev/null)
