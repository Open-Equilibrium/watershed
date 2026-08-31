pub(in crate::runtime) mod init;
pub(in crate::runtime) mod registry;
mod storage;

use core_script::RegistryBlockKind;

pub(in crate::runtime) const fn registry_directory(kind: RegistryBlockKind) -> &'static str {
    match kind {
        RegistryBlockKind::Tool => "tools",
        RegistryBlockKind::Instruction => "instructions",
        RegistryBlockKind::Phase => "phases",
        RegistryBlockKind::Flow => "flows",
    }
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(in crate::runtime) use init::DEFAULT_REGISTRY_ROOT;
pub use init::initialize_global_config;
pub use registry::{create_global_registry_block, validate_global_registry};
pub use storage::read_authoring_file;

#[cfg(test)]
pub(crate) use init::{set_init_post_marker_removal_observer, set_init_serialization_observer};
#[cfg(test)]
pub(crate) use registry::set_create_post_validation_observer;
#[cfg(test)]
pub(crate) use storage::{set_authoring_post_publication_failure, write_new_file_for_test};
