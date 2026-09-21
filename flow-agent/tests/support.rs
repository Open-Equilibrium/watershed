#[path = "support/replay.rs"]
mod replay;
#[path = "support/workspace.rs"]
mod workspace;

#[allow(unused_imports)]
pub(crate) use replay::{remove_replay_segments, write_sized_conversation_replay};
#[allow(unused_imports)]
pub(crate) use workspace::{
    TempWorkspace, absent_global_home, copy_dir, empty_workspace, empty_workspace_under,
    expected_stream, fixture_dir, run_current_test_isolated_session_home, session_home_path,
    stream_prefix, workspace_copy, workspace_log_dir, workspace_session_dir,
};

pub(crate) fn current_test_name() -> String {
    std::thread::current()
        .name()
        .expect("test thread has a name")
        .to_owned()
}
