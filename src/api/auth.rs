use crate::error::CoreError;
use axum::http::Method;
use subtle::ConstantTimeEq;

pub fn backend_authorized(value: Option<&str>, backend: &str) -> bool {
    matches_token(value, backend)
}

fn matches_token(value: Option<&str>, expected: &str) -> bool {
    !expected.is_empty()
        && value
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|token| bool::from(token.as_bytes().ct_eq(expected.as_bytes())))
}

pub fn authorize_http(
    value: Option<&str>,
    method: &Method,
    path: &str,
    query: Option<&str>,
    backend: &str,
    covenant: Option<&str>,
) -> Result<(), CoreError> {
    if backend_authorized(value, backend) {
        return Ok(());
    }
    if !covenant.is_some_and(|token| matches_token(value, token)) {
        return Err(CoreError::Unauthorized);
    }
    if query.is_none() && covenant_route(method, path) {
        return Ok(());
    }
    Err(CoreError::Policy(
        "Covenant credential cannot access this core route".into(),
    ))
}

fn covenant_route(method: &Method, path: &str) -> bool {
    if method == Method::POST {
        return path == "/v1/chat/completions";
    }
    if method != Method::GET {
        return false;
    }
    path == "/v1/models"
        || path
            .strip_prefix("/v1/runs/")
            .is_some_and(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BACK: &str = "fixture-backend-distinct-token-123456";
    const COV: &str = "fixture-covenant-distinct-token-12345";
    fn check(method: Method, path: &str, query: Option<&str>) -> Result<(), CoreError> {
        authorize_http(
            Some(&format!("Bearer {COV}")),
            &method,
            path,
            query,
            BACK,
            Some(COV),
        )
    }
    #[test]
    fn covenant_has_only_three_endpoint_permissions() {
        assert!(check(Method::GET, "/v1/models", None).is_ok());
        assert!(check(Method::POST, "/v1/chat/completions", None).is_ok());
        assert!(
            check(
                Method::GET,
                "/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0",
                None
            )
            .is_ok()
        );
        for path in [
            "/v1/runs",
            "/v1/realtime/session",
            "/v1/diarization",
            "/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0/cancel",
            "/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0/steer",
        ] {
            assert!(matches!(
                check(Method::POST, path, None),
                Err(CoreError::Policy(_))
            ));
        }
        for path in [
            "/v1/runs",
            "/v1/diarization",
            "/v1/runs/not-a-uuid",
            "/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0/events",
        ] {
            assert!(matches!(
                check(Method::GET, path, None),
                Err(CoreError::Policy(_))
            ));
        }
        assert!(check(Method::GET, "/v1/models", Some("after=0")).is_err());
        assert!(check(Method::DELETE, "/v1/models", None).is_err());
    }
    #[test]
    fn credentials_are_not_interchangeable_or_optional_fallbacks() {
        assert!(!backend_authorized(Some(&format!("Bearer {COV}")), BACK));
        assert!(!backend_authorized(Some(BACK), BACK));
        assert!(!backend_authorized(None, BACK));
        assert!(backend_authorized(Some(&format!("Bearer {BACK}")), BACK));
        assert!(matches!(
            authorize_http(
                Some(&format!("Bearer {COV}")),
                &Method::GET,
                "/v1/models",
                None,
                BACK,
                None
            ),
            Err(CoreError::Unauthorized)
        ));
    }
}
