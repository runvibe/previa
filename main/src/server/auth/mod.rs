pub mod config;
pub mod passwords;
pub mod permissions;
pub mod tokens;

use config::{AuthConfig, AuthMode};
use permissions::Role;
use tokens::{ApiTokenIssuer, JwtIssuer};

#[derive(Debug, Clone)]
pub struct AuthRuntime {
    pub config: AuthConfig,
    pub jwt: Option<JwtIssuer>,
    pub api_tokens: Option<ApiTokenIssuer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub subject: String,
    pub username: String,
    pub role: Role,
    pub source: PrincipalSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalSource {
    Env,
    Database,
    ApiToken,
    Anonymous,
}

pub fn anonymous_principal() -> Principal {
    Principal {
        subject: "anonymous".to_owned(),
        username: "anonymous".to_owned(),
        role: Role::Anonymous,
        source: PrincipalSource::Anonymous,
    }
}

pub fn anonymous_full_access_principal() -> Principal {
    Principal {
        subject: "anonymous".to_owned(),
        username: "anonymous".to_owned(),
        role: Role::Anonymous,
        source: PrincipalSource::Env,
    }
}

impl AuthRuntime {
    pub fn anonymous() -> Self {
        Self {
            config: AuthConfig::default(),
            jwt: None,
            api_tokens: None,
        }
    }

    pub fn from_config(config: AuthConfig) -> Result<Self, tokens::TokenError> {
        let jwt = match (config.mode(), config.jwt_secret.as_deref()) {
            (AuthMode::Protected, Some(secret)) => Some(JwtIssuer::new(
                "previa-main",
                secret,
                config.jwt_ttl_seconds,
            )?),
            _ => None,
        };
        let api_tokens = config
            .jwt_secret
            .as_deref()
            .map(ApiTokenIssuer::new)
            .transpose()?;
        Ok(Self {
            config,
            jwt,
            api_tokens,
        })
    }
}

impl Default for AuthRuntime {
    fn default() -> Self {
        Self::anonymous()
    }
}

#[cfg(test)]
mod tests {
    use super::config::{AuthConfig, AuthMode};
    use super::passwords::{hash_password, verify_password};
    use super::permissions::{Permission, Role};
    use super::tokens::{ApiTokenIssuer, JwtIssuer};

    #[test]
    fn anonymous_mode_is_default_and_full_access() {
        let config = AuthConfig::from_env_values(&[]).expect("auth config");

        assert_eq!(config.mode(), AuthMode::AnonymousFullAccess);
        assert!(Role::Anonymous.allows(Permission::ManageUsers));
        assert!(Role::Anonymous.allows(Permission::ManageRunners));
    }

    #[test]
    fn protected_mode_requires_root_and_jwt_secret() {
        let err = AuthConfig::from_env_values(&[("PREVIA_AUTH_ANONYMOUS", "false")])
            .expect_err("protected config should reject missing secrets");

        assert!(err.to_string().contains("PREVIA_ROOT_USERNAME"));
        assert!(err.to_string().contains("PREVIA_ROOT_PASSWORD"));
        assert!(err.to_string().contains("PREVIA_JWT_SECRET"));
    }

    #[test]
    fn editor_can_execute_but_cannot_manage_runners() {
        assert!(Role::Editor.allows(Permission::WriteProjects));
        assert!(Role::Editor.allows(Permission::RunExecutions));
        assert!(Role::Editor.allows(Permission::ReadRunners));
        assert!(!Role::Editor.allows(Permission::ManageRunners));
    }

    #[test]
    fn operator_can_run_but_not_edit_projects() {
        assert!(Role::Operator.allows(Permission::ReadProjects));
        assert!(Role::Operator.allows(Permission::RunExecutions));
        assert!(!Role::Operator.allows(Permission::WriteProjects));
    }

    #[test]
    fn hashes_passwords_without_storing_plaintext() {
        let hash = hash_password("secret").expect("hash password");

        assert_ne!(hash, "secret");
        assert!(verify_password("secret", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn jwt_round_trips_claims_and_rejects_tampering() {
        let issuer = JwtIssuer::new("issuer", "super-secret", 3600).expect("issuer");
        let token = issuer
            .issue("usr_123", "ana", Role::Editor, "database")
            .expect("issue jwt");

        let claims = issuer.verify(&token).expect("verify jwt");
        assert_eq!(claims.sub, "usr_123");
        assert_eq!(claims.username, "ana");
        assert_eq!(claims.role, Role::Editor);
        assert_eq!(claims.source, "database");

        let mut tampered = token.clone();
        tampered.push('x');
        assert!(issuer.verify(&tampered).is_err());
    }

    #[test]
    fn api_tokens_are_shown_once_and_verified_by_hash() {
        let issuer = ApiTokenIssuer::new("hash-secret").expect("token issuer");
        let issued = issuer.issue().expect("issue api token");

        assert!(issued.raw.starts_with("pvk_"));
        assert!(issued.prefix.starts_with("pvk_"));
        assert_ne!(issued.raw, issued.hash);
        assert!(issuer.verify(&issued.raw, &issued.hash));
        assert!(!issuer.verify("pvk_wrong", &issued.hash));
    }
}
