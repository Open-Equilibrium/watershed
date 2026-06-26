//! Loop Agent core M0 walking skeleton.

pub const LOCAL_SESSION_DIR: &str = ".loop/sessions";
pub const LOCAL_LOG_DIR: &str = ".loop/logs";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSurface {
    HumanCli,
    JsonlEventStream,
    LocalSessionLog,
    TailReplayResume,
    DesignedRpc,
    FutureEmbeddedCoreApi,
}

pub fn m1_runtime_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::HumanCli,
        RuntimeSurface::JsonlEventStream,
        RuntimeSurface::LocalSessionLog,
        RuntimeSurface::TailReplayResume,
    ]
}

pub fn designed_future_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::DesignedRpc,
        RuntimeSurface::FutureEmbeddedCoreApi,
    ]
}

pub fn validate_session_id(session_id: &str) -> bool {
    proto::is_valid_session_id(session_id)
}

pub fn m0_runtime_notice() -> &'static str {
    "M0 defines Loop Agent contracts and fixtures; runtime execution lands in M1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m1_surfaces_exclude_rpc_and_embedding() {
        let m1 = m1_runtime_surfaces();

        assert!(m1.contains(&RuntimeSurface::HumanCli));
        assert!(m1.contains(&RuntimeSurface::JsonlEventStream));
        assert!(!m1.contains(&RuntimeSurface::DesignedRpc));
        assert!(!m1.contains(&RuntimeSurface::FutureEmbeddedCoreApi));
    }

    #[test]
    fn session_id_validation_uses_protocol_contract() {
        assert!(validate_session_id("hello001"));
        assert!(!validate_session_id("Hello001"));
        assert!(!validate_session_id("../hello001"));
    }
}
