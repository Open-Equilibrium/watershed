use flow_agent_core::RuntimeError;

pub(super) use crate::test_support::empty_workspace;

pub(super) fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn assert_usage(result: Result<impl Sized, RuntimeError>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("authoring grammar must reject invalid input"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, RuntimeError::Usage(_)) && error.to_string().contains(expected),
        "{error}"
    );
}
