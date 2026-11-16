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
