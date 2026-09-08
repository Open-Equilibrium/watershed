use crate::runtime::productive::{
    ensure_productive_execution_platform, ensure_productive_tool_execution_platform,
};

#[test]
fn productive_tool_platform_accepts_the_supported_native_host() {
    ensure_productive_execution_platform().expect("native host supports productive execution");
    ensure_productive_tool_execution_platform().expect("native host admits Tool preparation");
}
