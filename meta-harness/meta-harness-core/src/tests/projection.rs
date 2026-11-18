use serde_json::json;

use crate::t3::Projection;

#[test]
fn a_subagent_keeps_its_prompt_and_result_across_consecutive_omitted_updates() {
    let mut projection = Projection::default();
    let update = |payload| json!({"type": "subagent.updated", "payload": payload});
    projection.apply(&update(json!({
        "id": "s1", "prompt": "P", "result": "R", "progress": "0",
    })));
    for progress in ["1", "2"] {
        projection.apply(&update(json!({
            "id": "s1", "prompt": "", "promptOmitted": true, "result": null, "resultOmitted": true,
            "progress": progress,
        })));
        assert_eq!(
            projection.subagents,
            vec![json!({"id": "s1", "prompt": "P", "result": "R", "progress": progress})]
        );
    }
}

#[test]
fn a_queued_message_with_attachments_is_reported_and_a_running_one_is_not() {
    let mut projection = Projection {
        runs: vec![
            json!({"id": "r1", "status": "running", "userMessageId": "m1"}),
            json!({"id": "r2", "status": "queued", "userMessageId": "m2"}),
        ],
        messages: vec![
            json!({"id": "m1", "attachments": [{"id": "a1"}]}),
            json!({"id": "m2", "attachments": []}),
        ],
        ..Projection::default()
    };
    assert!(!projection.queued_with_attachments());
    projection.messages[1] = json!({"id": "m2", "attachments": [{"id": "a2"}]});
    assert!(projection.queued_with_attachments());
}
