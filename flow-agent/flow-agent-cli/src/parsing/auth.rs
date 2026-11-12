use flow_agent_core::{
    AuthLoginMode, OPENAI_CODEX_PROVIDER_ID, OPENAI_COMPATIBLE_PROVIDER_ID, RuntimeError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthProvider {
    OpenAiCodex,
    OpenAiCompatible,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AuthLogin {
    Subscription(AuthLoginMode),
    /// The base URL is an argument; the key itself is read from standard input.
    ApiKey {
        base_url: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AuthCommand {
    Login(AuthLogin),
    Status(AuthProvider),
    Logout(AuthProvider),
}

pub(super) fn usage_commands() -> [String; 6] {
    [
        format!("flow auth login {OPENAI_CODEX_PROVIDER_ID} <--browser|--device>"),
        format!(
            "flow auth login {OPENAI_COMPATIBLE_PROVIDER_ID} --base-url <URL> --api-key (the key is read from standard input)"
        ),
        format!("flow auth status {OPENAI_CODEX_PROVIDER_ID}"),
        format!("flow auth status {OPENAI_COMPATIBLE_PROVIDER_ID}"),
        format!("flow auth logout {OPENAI_CODEX_PROVIDER_ID}"),
        format!("flow auth logout {OPENAI_COMPATIBLE_PROVIDER_ID}"),
    ]
}

pub(crate) fn auth_args(args: &[String]) -> Result<AuthCommand, RuntimeError> {
    match args {
        [auth, login, provider, mode]
            if auth == "auth" && login == "login" && provider == OPENAI_CODEX_PROVIDER_ID =>
        {
            match mode.as_str() {
                "--browser" => Ok(AuthCommand::Login(AuthLogin::Subscription(
                    AuthLoginMode::Browser,
                ))),
                "--device" => Ok(AuthCommand::Login(AuthLogin::Subscription(
                    AuthLoginMode::Device,
                ))),
                _ => Err(RuntimeError::Usage(format!(
                    "unknown authentication login mode {mode:?}"
                ))),
            }
        }
        [auth, login, provider, base_flag, base_url, key_flag]
            if auth == "auth"
                && login == "login"
                && provider == OPENAI_COMPATIBLE_PROVIDER_ID
                && base_flag == "--base-url"
                && key_flag == "--api-key" =>
        {
            Ok(AuthCommand::Login(AuthLogin::ApiKey {
                base_url: base_url.clone(),
            }))
        }
        [auth, login, provider, base_flag, _, key_flag, _]
            if auth == "auth"
                && login == "login"
                && provider == OPENAI_COMPATIBLE_PROVIDER_ID
                && base_flag == "--base-url"
                && key_flag == "--api-key" =>
        {
            Err(RuntimeError::Usage(
                "the API key is read from standard input, not from arguments".to_owned(),
            ))
        }
        [auth, status, provider] if auth == "auth" && status == "status" => {
            Ok(AuthCommand::Status(auth_provider(provider)?))
        }
        [auth, logout, provider] if auth == "auth" && logout == "logout" => {
            Ok(AuthCommand::Logout(auth_provider(provider)?))
        }
        _ => Err(usage_error()),
    }
}

fn auth_provider(provider: &str) -> Result<AuthProvider, RuntimeError> {
    match provider {
        OPENAI_CODEX_PROVIDER_ID => Ok(AuthProvider::OpenAiCodex),
        OPENAI_COMPATIBLE_PROVIDER_ID => Ok(AuthProvider::OpenAiCompatible),
        _ => Err(usage_error()),
    }
}

fn usage_error() -> RuntimeError {
    RuntimeError::Usage(format!("usage: {}", usage_commands().join(" | ")))
}

#[cfg(test)]
mod tests {
    use super::{AuthCommand, AuthLogin, AuthLoginMode, AuthProvider, auth_args};
    use crate::parsing::strings;

    #[test]
    fn authentication_commands_have_one_exact_grammar() {
        assert_eq!(
            auth_args(&strings(&["auth", "login", "openai-codex", "--browser"]))
                .expect("browser login grammar is valid"),
            AuthCommand::Login(AuthLogin::Subscription(AuthLoginMode::Browser))
        );
        assert_eq!(
            auth_args(&strings(&["auth", "login", "openai-codex", "--device"]))
                .expect("device login grammar is valid"),
            AuthCommand::Login(AuthLogin::Subscription(AuthLoginMode::Device))
        );
        assert_eq!(
            auth_args(&strings(&["auth", "status", "openai-codex"]))
                .expect("status grammar is valid"),
            AuthCommand::Status(AuthProvider::OpenAiCodex)
        );
        assert_eq!(
            auth_args(&strings(&["auth", "logout", "openai-codex"]))
                .expect("logout grammar is valid"),
            AuthCommand::Logout(AuthProvider::OpenAiCodex)
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

    #[test]
    fn endpoint_login_takes_a_base_url_and_reads_the_key_from_stdin() {
        assert_eq!(
            auth_args(&strings(&[
                "auth",
                "login",
                "openai-compatible",
                "--base-url",
                "https://models.example.test/v1",
                "--api-key",
            ]))
            .expect("endpoint login grammar is valid"),
            AuthCommand::Login(AuthLogin::ApiKey {
                base_url: "https://models.example.test/v1".to_owned(),
            })
        );
        assert_eq!(
            auth_args(&strings(&["auth", "status", "openai-compatible"]))
                .expect("endpoint status grammar is valid"),
            AuthCommand::Status(AuthProvider::OpenAiCompatible)
        );
        assert_eq!(
            auth_args(&strings(&["auth", "logout", "openai-compatible"]))
                .expect("endpoint logout grammar is valid"),
            AuthCommand::Logout(AuthProvider::OpenAiCompatible)
        );
    }

    #[test]
    fn endpoint_login_rejects_a_key_argument_and_an_incomplete_grammar() {
        for args in [
            &["auth", "login", "openai-compatible"][..],
            &["auth", "login", "openai-compatible", "--api-key"][..],
            &[
                "auth",
                "login",
                "openai-compatible",
                "--base-url",
                "https://x",
            ][..],
            &[
                "auth",
                "login",
                "openai-compatible",
                "--base-url",
                "https://x",
                "--api-key",
                "sk-argv-fixture",
            ][..],
            &["auth", "login", "openai-codex", "--api-key"][..],
        ] {
            assert!(auth_args(&strings(args)).is_err(), "accepted {args:?}");
        }
    }
}
