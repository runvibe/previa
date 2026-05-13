use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    AnonymousFullAccess,
    Protected,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    mode: AuthMode,
    pub root_username: Option<String>,
    pub root_password: Option<String>,
    pub jwt_secret: Option<String>,
    pub jwt_ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfigError {
    missing: Vec<&'static str>,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self, AuthConfigError> {
        let values = std::env::vars()
            .filter(|(key, _)| key.starts_with("PREVIA_"))
            .collect::<BTreeMap<_, _>>();
        let borrowed = values
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        Self::from_env_values(&borrowed)
    }

    pub fn from_env_values(values: &[(&str, &str)]) -> Result<Self, AuthConfigError> {
        let lookup = values
            .iter()
            .map(|(key, value)| (*key, value.trim()))
            .collect::<BTreeMap<_, _>>();
        let anonymous = lookup
            .get("PREVIA_AUTH_ANONYMOUS")
            .map(|value| truthy(value))
            .unwrap_or(true);
        let mode = if anonymous {
            AuthMode::AnonymousFullAccess
        } else {
            AuthMode::Protected
        };
        let root_username = non_empty(lookup.get("PREVIA_ROOT_USERNAME").copied());
        let root_password = non_empty(lookup.get("PREVIA_ROOT_PASSWORD").copied());
        let jwt_secret = non_empty(lookup.get("PREVIA_JWT_SECRET").copied());
        let jwt_ttl_seconds = lookup
            .get("PREVIA_JWT_TTL_SECONDS")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(24 * 60 * 60);

        if mode == AuthMode::Protected {
            let mut missing = Vec::new();
            if root_username.is_none() {
                missing.push("PREVIA_ROOT_USERNAME");
            }
            if root_password.is_none() {
                missing.push("PREVIA_ROOT_PASSWORD");
            }
            if jwt_secret.is_none() {
                missing.push("PREVIA_JWT_SECRET");
            }
            if !missing.is_empty() {
                return Err(AuthConfigError { missing });
            }
        }

        Ok(Self {
            mode,
            root_username,
            root_password,
            jwt_secret,
            jwt_ttl_seconds,
        })
    }

    pub fn mode(&self) -> AuthMode {
        self.mode
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::AnonymousFullAccess,
            root_username: None,
            root_password: None,
            jwt_secret: None,
            jwt_ttl_seconds: 24 * 60 * 60,
        }
    }
}

impl fmt::Display for AuthConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "protected mode requires missing env vars: {}",
            self.missing.join(", ")
        )
    }
}

impl Error for AuthConfigError {}

fn truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
