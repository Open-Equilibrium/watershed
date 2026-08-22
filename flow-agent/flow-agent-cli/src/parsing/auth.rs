use flow_agent_core::{AuthLoginMode, OPENAI_CODEX_PROVIDER_ID, RuntimeError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthCommand {
    Login(AuthLoginMode),
    Status,
    Logout,
}

pub(super) fn usage_commands() -> [String; 3] {
    [
        format!("flow auth login {OPENAI_CODEX_PROVIDER_ID} <--browser|--device>"),
        format!("flow auth status {OPENAI_CODEX_PROVIDER_ID}"),
        format!("flow auth logout {OPENAI_CODEX_PROVIDER_ID}"),
    ]
}

pub(crate) fn auth_args(args: &[String]) -> Result<AuthCommand, RuntimeError> {
    match args {
        [auth, login, provider, mode]
            if auth == "auth" && login == "login" && provider == OPENAI_CODEX_PROVIDER_ID =>
        {
            match mode.as_str() {
                "--browser" => Ok(AuthCommand::Login(AuthLoginMode::Browser)),
                "--device" => Ok(AuthCommand::Login(AuthLoginMode::Device)),
                _ => Err(RuntimeError::Usage(format!(
                    "unknown authentication login mode {mode:?}"
                ))),
            }
        }
        [auth, status, provider]
            if auth == "auth" && status == "status" && provider == OPENAI_CODEX_PROVIDER_ID =>
        {
            Ok(AuthCommand::Status)
        }
        [auth, logout, provider]
            if auth == "auth" && logout == "logout" && provider == OPENAI_CODEX_PROVIDER_ID =>
        {
            Ok(AuthCommand::Logout)
        }
        _ => Err(RuntimeError::Usage(format!(
            "usage: {}",
            usage_commands().join(" | ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthCommand, AuthLoginMode, auth_args};
    use crate::parsing::strings;

    #[test]
    fn authentication_commands_have_one_exact_grammar() {
        assert_eq!(
            auth_args(&strings(&["auth", "login", "openai-codex", "--browser"]))
                .expect("browser login grammar is valid"),
            AuthCommand::Login(AuthLoginMode::Browser)
        );
        assert_eq!(
            auth_args(&strings(&["auth", "login", "openai-codex", "--device"]))
                .expect("device login grammar is valid"),
            AuthCommand::Login(AuthLoginMode::Device)
        );
        assert_eq!(
            auth_args(&strings(&["auth", "status", "openai-codex"]))
                .expect("status grammar is valid"),
            AuthCommand::Status
        );
        assert_eq!(
            auth_args(&strings(&["auth", "logout", "openai-codex"]))
                .expect("logout grammar is valid"),
            AuthCommand::Logout
        );
    }

    #[test]
    fn authentication_commands_reject_implicit_modes_and_extra_arguments() {
        for args in [
            &["auth", "login", "openai-codex"][..],
            &["auth", "login", "openai-codex", "--browser", "extra"][..],
            &["auth", "login", "other", "--browser"][..],
            &["auth", "status", "openai-codex", "extra"][..],
            &["auth", "logout", "other"][..],
        ] {
            assert!(auth_args(&strings(args)).is_err(), "accepted {args:?}");
        }
    }
}
