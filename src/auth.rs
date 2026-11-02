//! API authentication.

use axum::http::{HeaderMap, header};
use subtle::ConstantTimeEq;
use thiserror::Error;

use crate::config::{AuthMode, Config};

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing bearer token")]
    Missing,
    #[error("invalid bearer token")]
    Invalid,
    #[error("bearer mode has no configured token")]
    Misconfigured,
}

#[derive(Debug)]
pub enum Auth {
    Bearer { token: String },
    Disabled,
}

impl Auth {
    pub fn new(config: &Config, token: Option<String>) -> Result<Self, AuthError> {
        match config.auth.mode {
            AuthMode::Bearer => {
                let token = token.ok_or(AuthError::Misconfigured)?;
                Ok(Self::Bearer { token })
            }
            AuthMode::None => Ok(Self::Disabled),
        }
    }

    pub fn check(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        let Self::Bearer { token } = self else {
            return Ok(());
        };
        let mut values = headers.get_all(header::AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return Err(AuthError::Missing);
        };
        if values.next().is_some() {
            return Err(AuthError::Invalid);
        }
        let value = value.to_str().map_err(|_| AuthError::Invalid)?;
        let (scheme, provided) = value.split_once(' ').ok_or(AuthError::Invalid)?;
        if !scheme.eq_ignore_ascii_case("bearer") || provided.is_empty() {
            return Err(AuthError::Invalid);
        }
        if constant_time_eq(token.as_bytes(), provided.as_bytes()) {
            Ok(())
        } else {
            Err(AuthError::Invalid)
        }
    }
}

fn constant_time_eq(expected: &[u8], provided: &[u8]) -> bool {
    if expected.len() != provided.len() {
        return false;
    }
    bool::from(expected.ct_eq(provided))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn auth() -> Auth {
        let mut config: Config = toml::from_str("").unwrap();
        config.auth.mode = AuthMode::Bearer;
        Auth::new(&config, Some("0123456789abcdef".into())).unwrap()
    }

    fn headers(value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(header::AUTHORIZATION, value.parse().unwrap());
        map
    }

    #[test]
    fn accepts_correct_token() {
        assert!(auth().check(&headers("Bearer 0123456789abcdef")).is_ok());
        assert!(auth().check(&headers("bearer 0123456789abcdef")).is_ok());
    }

    #[test]
    fn rejects_missing_or_wrong_token() {
        assert!(matches!(
            auth().check(&HeaderMap::new()),
            Err(AuthError::Missing)
        ));
        assert!(matches!(
            auth().check(&headers("Bearer nope")),
            Err(AuthError::Invalid)
        ));
        assert!(matches!(
            auth().check(&headers("Basic 0123456789abcdef")),
            Err(AuthError::Invalid)
        ));
    }

    #[test]
    fn rejects_duplicate_authorization_headers() {
        let mut map = headers("Bearer 0123456789abcdef");
        map.append(
            header::AUTHORIZATION,
            "Bearer 0123456789abcdef".parse().unwrap(),
        );
        assert!(matches!(auth().check(&map), Err(AuthError::Invalid)));
    }

    #[test]
    fn disabled_auth_allows_everything() {
        let auth = Auth::Disabled;
        assert!(auth.check(&HeaderMap::new()).is_ok());
    }
}
