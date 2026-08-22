use super::{
    support::{completed_phase_result, write_registry_definition},
    test_support::workspace_copy,
};
use crate::runtime::{
    session::run_flow_with_root_input, types::EmitMode, validate::validate_session_log_text,
};
use proto::EventType;

#[test]
fn custom_m11_flow_composes_parameters_loops_transitions_types_and_subflows() {
    let workspace = workspace_copy("smoke-flow");
    write_registry_definition(
        &workspace,
        "instructions",
        "parameterized",
        r#"instruction:
  id: parameterized
  name: Parameterized
  prompt: Review {{project}}.
  parameters:
    - name: project
      value_contract:
        type: string
        max_length: 32
"#,
    );
    write_registry_definition(
        &workspace,
        "instructions",
        "loop-values",
        r#"instruction:
  id: loop-values
  name: LoopValues
  prompt: 'fixture-tool-request: none fixture-results: [{"type":"string","value":"again"},{"type":"string","value":"done"}]'
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "parameterized-input",
        r#"phase:
  id: parameterized-input
  name: ParameterizedInput
  instruction_refs: [parameterized]
  tool_refs: []
  output:
    type: map
    fields:
      - name: project
        required: true
        value_contract:
          type: string
          max_length: 32
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "repeat",
        r#"phase:
  id: repeat
  name: Repeat
  instruction_refs: [loop-values]
  tool_refs: []
  output:
    type: string
  loop:
    max_iterations: 2
    until:
      path: []
      equals:
        type: string
        value: done
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "must-skip",
        r#"phase:
  id: must-skip
  name: MustSkip
  instruction_refs: []
  tool_refs: []
  output:
    type: boolean
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "boolean-value",
        r#"phase:
  id: boolean-value
  name: BooleanValue
  instruction_refs: []
  tool_refs: []
  output:
    type: boolean
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "integer-value",
        r#"phase:
  id: integer-value
  name: IntegerValue
  instruction_refs: []
  tool_refs: []
  output:
    type: integer
    min: 4
    max: 8
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "string-value",
        r#"phase:
  id: string-value
  name: StringValue
  instruction_refs: []
  tool_refs: []
  output:
    type: string
    max_length: 3
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "list-value",
        r#"phase:
  id: list-value
  name: ListValue
  instruction_refs: []
  tool_refs: []
  output:
    type: list
    items:
      type: boolean
    max_items: 2
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "map-value",
        r#"phase:
  id: map-value
  name: MapValue
  instruction_refs: []
  tool_refs: []
  output:
    type: map
    fields:
      - name: flag
        required: true
        value_contract:
          type: boolean
      - name: count
        required: true
        value_contract:
          type: integer
          min: 2
      - name: text
        required: true
        value_contract:
          type: string
          max_length: 2
      - name: items
        required: true
        value_contract:
          type: list
          items:
            type: boolean
      - name: object
        required: true
        value_contract:
          type: session-object
      - name: omitted
        required: false
        value_contract:
          type: string
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "object-value",
        r#"phase:
  id: object-value
  name: ObjectValue
  instruction_refs: []
  tool_refs: []
  output:
    type: session-object
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "composite",
        r#"phase:
  id: composite
  name: Composite
  instruction_refs: []
  tool_refs: []
  phase_refs: [boolean-value, integer-value, string-value, list-value, map-value, object-value]
  output:
    type: session-object
  result_from: object-value
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "subflow-leaf",
        r#"phase:
  id: subflow-leaf
  name: SubflowLeaf
  instruction_refs: []
  tool_refs: []
  output:
    type: boolean
"#,
    );
    write_registry_definition(
        &workspace,
        "flows",
        "custom-child",
        r#"flow:
  id: custom-child
  name: CustomChild
  phase_refs: [subflow-leaf]
  subflow_refs: []
"#,
    );
    write_registry_definition(
        &workspace,
        "flows",
        "custom-root",
        r#"flow:
  id: custom-root
  name: CustomRoot
  phase_refs: [parameterized-input, repeat, must-skip, composite]
  subflow_refs: [custom-child]
  transitions:
    - from_phase_ref: repeat
      to_phase_ref: composite
      when:
        path: []
        equals:
          type: string
          value: done
"#,
    );

    let output = run_flow_with_root_input(
        &workspace,
        "custom-root",
        core_script::FlowValue::Map(std::collections::BTreeMap::from([(
            "project".to_owned(),
            core_script::FlowValue::String("watershed".to_owned()),
        )])),
        EmitMode::Jsonl,
    )
    .expect("custom M1.1 Flow completes");
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("custom M1.1 event stream validates");

    assert!(!output.failed);
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.event_type == EventType::PhaseEntered && event.payload["phase_id"] == "repeat"
            })
            .count(),
        2
    );
    assert!(
        !events
            .iter()
            .any(|event| event.payload["phase_id"] == "must-skip")
    );
    assert_eq!(
        completed_phase_result(&events, "parameterized-input"),
        &serde_json::json!({"type":"map","value":{"project":{"type":"string","value":"watershed"}}})
    );
    assert_eq!(
        completed_phase_result(&events, "integer-value"),
        &serde_json::json!({"type":"integer","value":"4"})
    );
    assert_eq!(
        completed_phase_result(&events, "string-value"),
        &serde_json::json!({"type":"string","value":"hel"})
    );
    assert_eq!(
        completed_phase_result(&events, "list-value"),
        &serde_json::json!({"type":"list","value":[]})
    );
    assert_eq!(
        completed_phase_result(&events, "map-value"),
        &serde_json::json!({
            "type":"map",
            "value":{
                "count":{"type":"integer","value":"2"},
                "flag":{"type":"boolean","value":true},
                "items":{"type":"list","value":[]},
                "object":{"type":"session-object","value":format!("session-object:sha256:{}", "0".repeat(64))},
                "text":{"type":"string","value":"he"}
            }
        })
    );
    assert!(events.iter().any(|event| {
        event.event_type == EventType::FlowCompleted
            && event.payload["flow_definition_id"] == "custom-child"
    }));
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );
}
