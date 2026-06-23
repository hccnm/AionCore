#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

use aionui_common::ApiError;
use aionui_db::{IExternalIdentityRepository, IPlatformUserRepository, IUserRepository};

use crate::JwtService;
use crate::extract::extract_token_from_headers;

/// Authenticated user injected into request extensions by the auth middleware.
///
/// Route handlers extract this from `request.extensions()` to identify
/// the current user.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// User ID from the database.
    pub id: String,
    /// Username.
    pub username: String,
}

#[derive(Clone, Debug)]
pub struct GatewayAuthConfig {
    pub app_id: String,
    pub app_secret: String,
    pub provider: String,
    pub timestamp_skew_seconds: u64,
}

/// Shared state for the authentication middleware.
#[derive(Clone)]
pub struct AuthState {
    pub jwt_service: Arc<JwtService>,
    pub user_repo: Arc<dyn IUserRepository>,
    pub platform_user_repo: Option<Arc<dyn IPlatformUserRepository>>,
    pub external_identity_repo: Option<Arc<dyn IExternalIdentityRepository>>,
    pub gateway_auth: Option<GatewayAuthConfig>,
    /// When `true`, skip JWT verification and inject a fixed default user.
    pub local: bool,
}

/// Authentication middleware that verifies JWT tokens and injects `CurrentUser`.
///
/// Flow:
/// 1. Extract bearer token from `Authorization` header or `aionui-session` cookie
/// 2. Verify JWT signature, expiration, and blacklist
/// 3. Look up user in the database to ensure they still exist
/// 4. Insert [`CurrentUser`] into request extensions
///
/// Returns HTTP 401 for authentication failures.
///
/// Use with `axum::middleware::from_fn_with_state`.
pub async fn auth_middleware(
    State(state): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // In local mode, skip JWT verification and inject a fixed default user.
    if state.local {
        request.extensions_mut().insert(CurrentUser {
            id: "system_default_user".to_string(),
            username: "system_default_user".to_string(),
        });
        return Ok(next.run(request).await);
    }

    if let Some(current_user) = try_gateway_current_user(&state, request.headers()).await? {
        request.extensions_mut().insert(current_user);
        return Ok(next.run(request).await);
    }

    let token = extract_token_from_headers(request.headers())
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".into()))?;

    let payload = state.jwt_service.verify(&token).map_err(|e| {
        tracing::debug!("Token verification failed: {e}");
        ApiError::Unauthorized("Invalid or expired token".into())
    })?;

    let current_user = if let Some(repo) = &state.platform_user_repo {
        let user = repo
            .find_by_id(&payload.user_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "platform auth middleware user lookup failed");
                ApiError::Internal("Authentication service unavailable".into())
            })?
            .ok_or_else(|| ApiError::Unauthorized("Invalid authentication subject".into()))?;
        if user.status != "enabled" {
            return Err(ApiError::Unauthorized("User is disabled".into()));
        }
        CurrentUser {
            id: user.id,
            username: user
                .username
                .or(user.phone)
                .or(user.display_name)
                .unwrap_or_else(|| payload.username.clone()),
        }
    } else {
        let user = state
            .user_repo
            .find_by_id(&payload.user_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "auth middleware user lookup failed");
                ApiError::Internal("Authentication service unavailable".into())
            })?
            .ok_or_else(|| ApiError::Unauthorized("Invalid authentication subject".into()))?;

        CurrentUser {
            id: user.id,
            username: user.username,
        }
    };

    request.extensions_mut().insert(current_user);

    Ok(next.run(request).await)
}

async fn try_gateway_current_user(state: &AuthState, headers: &HeaderMap) -> Result<Option<CurrentUser>, ApiError> {
    let Some(config) = &state.gateway_auth else {
        return Ok(None);
    };
    if !is_gateway_request(headers) {
        return Ok(None);
    }
    let Some(platform_user_repo) = &state.platform_user_repo else {
        return Err(ApiError::Unauthorized("Gateway authentication is not available".into()));
    };
    let Some(external_identity_repo) = &state.external_identity_repo else {
        return Err(ApiError::Unauthorized(
            "Gateway identity mapping is not available".into(),
        ));
    };

    let app_id = header_value(headers, "x-gateway-app-id")
        .ok_or_else(|| ApiError::Unauthorized("Missing gateway app id".into()))?;
    if !constant_time_eq(app_id.as_bytes(), config.app_id.as_bytes()) {
        return Err(ApiError::Unauthorized("Invalid gateway app id".into()));
    }

    let bearer = bearer_token(headers).ok_or_else(|| ApiError::Unauthorized("Missing gateway app secret".into()))?;
    if !constant_time_eq(bearer.as_bytes(), config.app_secret.as_bytes()) {
        return Err(ApiError::Unauthorized("Invalid gateway app secret".into()));
    }

    let external_user_id = header_value(headers, "x-gateway-user-id")
        .ok_or_else(|| ApiError::Unauthorized("Missing gateway user id".into()))?;
    let timestamp = header_value(headers, "x-gateway-timestamp")
        .ok_or_else(|| ApiError::Unauthorized("Missing gateway timestamp".into()))?;
    verify_gateway_timestamp(timestamp, config.timestamp_skew_seconds)?;

    let signature = header_value(headers, "x-gateway-user-id-signature")
        .ok_or_else(|| ApiError::Unauthorized("Missing gateway signature".into()))?;
    let expected = gateway_user_signature(&config.app_id, external_user_id, timestamp, &config.app_secret);
    if !constant_time_eq_ignore_ascii_case(signature.as_bytes(), expected.as_bytes()) {
        return Err(ApiError::Unauthorized("Invalid gateway user signature".into()));
    }

    let identity = external_identity_repo
        .find_identity(&config.provider, &config.app_id, external_user_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "gateway identity lookup failed");
            ApiError::Internal("Authentication service unavailable".into())
        })?
        .ok_or_else(|| ApiError::Unauthorized("Gateway user is not synced".into()))?;

    let user = platform_user_repo
        .find_by_id(&identity.user_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "gateway user lookup failed");
            ApiError::Internal("Authentication service unavailable".into())
        })?
        .ok_or_else(|| ApiError::Unauthorized("Gateway user is not synced".into()))?;
    if user.status != "enabled" {
        return Err(ApiError::Unauthorized("User is disabled".into()));
    }

    Ok(Some(CurrentUser {
        id: user.id,
        username: user
            .username
            .or(user.phone)
            .or(user.display_name)
            .unwrap_or_else(|| external_user_id.to_owned()),
    }))
}

fn is_gateway_request(headers: &HeaderMap) -> bool {
    header_value(headers, "x-gateway-request-from") == Some("app-gateway")
        && header_value(headers, "x-gateway-proxy-request") == Some("true")
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").filter(|token| !token.is_empty())
}

fn verify_gateway_timestamp(timestamp: &str, skew_seconds: u64) -> Result<(), ApiError> {
    let gateway_time = timestamp
        .parse::<u64>()
        .map_err(|_| ApiError::Unauthorized("Invalid gateway timestamp".into()))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Internal("System clock before UNIX_EPOCH".into()))?
        .as_secs();
    let delta = now.abs_diff(gateway_time);
    if delta <= skew_seconds {
        Ok(())
    } else {
        Err(ApiError::Unauthorized("Expired gateway timestamp".into()))
    }
}

pub fn gateway_user_signature(app_id: &str, user_id: &str, timestamp: &str, app_secret: &str) -> String {
    let canonical = format!("{app_id}:{user_id}:{timestamp}:{app_secret}");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

fn constant_time_eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0).to_ascii_lowercase();
        let right_byte = right.get(index).copied().unwrap_or(0).to_ascii_lowercase();
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

/// Local-mode authentication middleware that skips JWT verification.
///
/// Injects a fixed `CurrentUser` with id and username `system_default_user`.
/// Used when the server runs as an embedded subprocess inside Electron.
pub async fn local_auth_middleware(mut request: Request, next: Next) -> Response {
    request.extensions_mut().insert(CurrentUser {
        id: "system_default_user".to_string(),
        username: "system_default_user".to_string(),
    });
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    async fn echo_user(request: Request<Body>) -> String {
        let user = request.extensions().get::<CurrentUser>().unwrap();
        format!("{}:{}", user.id, user.username)
    }

    #[tokio::test]
    async fn test_local_auth_middleware_injects_default_user() {
        let app = Router::new()
            .route("/test", get(echo_user))
            .route_layer(axum::middleware::from_fn(local_auth_middleware));

        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "system_default_user:system_default_user"
        );
    }
}
