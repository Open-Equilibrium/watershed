mod own_script;
mod script_output;

pub use own_script::plan_own_script;
#[cfg(test)]
pub use own_script::{compile_own_script_operations, evaluate_script_command, script_redirection};
#[cfg(test)]
pub use script_output::{
    anchored_workspace_write_path, normalize_script_write_target, replace_script_output_atomically,
    set_script_output_cleanup_error_once, set_script_output_cleanup_errors,
    set_script_output_publish_observer, validate_script_write_target,
};
pub use script_output::{preflight_own_script_outputs, write_script_output};
