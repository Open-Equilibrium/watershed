#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{copy_dir, expected_stream, fixture_dir, workspace_copy};

include!("tests/support.rs");
include!("tests/helpers.rs");

mod surface_contracts {
    use super::*;
    include!("tests/surface_contracts.rs");
}

mod registry_runtime {
    use super::*;
    include!("tests/registry_runtime.rs");
}

mod session_listing {
    use super::*;
    include!("tests/session_listing.rs");
}

mod sandbox {
    use super::*;
    include!("tests/sandbox.rs");
}

mod session_logs {
    use super::*;
    include!("tests/session_logs.rs");
}

mod tail {
    use super::*;
    include!("tests/tail.rs");
}

mod fs_guards {
    use super::*;
    include!("tests/fs_guards.rs");
}

mod protocol {
    use super::*;
    include!("tests/protocol.rs");
}

mod workspace_security {
    use super::*;
    include!("tests/workspace_security.rs");
}

mod performance {
    use super::*;
    include!("tests/performance.rs");
}

mod context {
    use super::*;
    include!("tests/context.rs");
}

mod event_writer {
    use super::*;
    include!("tests/event_writer.rs");
}
