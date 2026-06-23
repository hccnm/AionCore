use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::routing::{get, post};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use tower::ServiceExt;

use aionui_auth::{
    AuthState, CookieConfig, CurrentUser, GatewayAuthConfig, JwtService, RateLimiter, TokenPayload,
    api_rate_limit_middleware, auth_middleware, auth_rate_limit_middleware, authenticated_action_rate_limit_middleware,
    csrf_middleware, gateway_user_signature, security_headers_middleware,
};
use aionui_db::{
    CreatePlatformUserParams, DbError, ExternalIdentityRow, IExternalIdentityRepository, IPlatformUserRepository,
    IUserRepository, PlatformUserRow, SqliteUserRepository, UpsertExternalIdentityParams, init_database_memory,
};

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

// ============================================================
// T12.1 — Security response headers
// ============================================================

#[tokio::test]
async fn t12_1_security_headers_on_get() {
    let app = Router::new()
        .route("/test", get(|| async { "ok" }))
        .layer(middleware::from_fn(security_headers_middleware));

    let resp = app
        .oneshot(Request::get("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
    assert_eq!(resp.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(resp.headers().get("x-xss-protection").unwrap(), "1; mode=block");
    assert_eq!(
        resp.headers().get("referrer-policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
}

// ============================================================
// T12.2 — CSRF protection
// ============================================================

fn csrf_app() -> Router {
    let config = Arc::new(CookieConfig {
        secure: false,
        same_site: "Lax",
    });
    Router::new()
        .route("/api/test", post(|| async { "ok" }))
        .route("/login", post(|| async { "logged in" }))
        .route("/api/auth/qr-login", post(|| async { "qr ok" }))
        .route("/get-test", get(|| async { "get ok" }))
        .layer(middleware::from_fn_with_state(config, csrf_middleware))
}

#[tokio::test]
async fn t12_2_get_requests_bypass_csrf() {
    let app = csrf_app();
    let resp = app
        .oneshot(Request::get("/get-test").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn t12_2_post_without_csrf_token_rejected() {
    let app = csrf_app();
    let resp = app
        .oneshot(Request::post("/api/test").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 403);
}

#[tokio::test]
async fn t12_2_post_with_matching_csrf_tokens_accepted() {
    let app = csrf_app();
    let token = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let resp = app
        .oneshot(
            Request::post("/api/test")
                .header("cookie", format!("aionui-csrf-token={token}"))
                .header("x-csrf-token", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn t12_2_post_with_mismatched_csrf_tokens_rejected() {
    let app = csrf_app();
    let resp = app
        .oneshot(
            Request::post("/api/test")
                .header("cookie", "aionui-csrf-token=token_a")
                .header("x-csrf-token", "token_b")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 403);
}

// ============================================================
// Auth middleware
// ============================================================

async fn auth_app(jwt_service: Arc<JwtService>) -> Router {
    let db = init_database_memory().await.unwrap();
    let user_repo = Arc::new(SqliteUserRepository::new(db.pool().clone())) as Arc<dyn IUserRepository>;
    protected_auth_app(jwt_service, user_repo)
}

fn protected_auth_app(jwt_service: Arc<JwtService>, user_repo: Arc<dyn IUserRepository>) -> Router {
    let state = AuthState {
        jwt_service,
        user_repo,
        platform_user_repo: None,
        external_identity_repo: None,
        gateway_auth: None,
        local: false,
    };

    Router::new()
        .route("/protected", get(|| async { "ok" }))
        .route_layer(middleware::from_fn_with_state(state, auth_middleware))
}

fn gateway_auth_app(
    platform_user_repo: Arc<dyn IPlatformUserRepository>,
    external_identity_repo: Arc<dyn IExternalIdentityRepository>,
    skew: u64,
) -> Router {
    let state = AuthState {
        jwt_service: Arc::new(JwtService::new("gateway_test_secret".into())),
        user_repo: Arc::new(FakeLegacyUserRepo),
        platform_user_repo: Some(platform_user_repo),
        external_identity_repo: Some(external_identity_repo),
        gateway_auth: Some(GatewayAuthConfig {
            app_id: "app_aion".into(),
            app_secret: "secret_123".into(),
            provider: "modo_open_platform".into(),
            timestamp_skew_seconds: skew,
        }),
        local: false,
    };

    Router::new()
        .route("/protected", get(echo_current_user))
        .route_layer(middleware::from_fn_with_state(state, auth_middleware))
}

async fn echo_current_user(request: Request<Body>) -> String {
    let user = request.extensions().get::<CurrentUser>().unwrap();
    format!("{}:{}", user.id, user.username)
}

fn gateway_request(signature: &str, secret: &str, timestamp: &str) -> Request<Body> {
    Request::get("/protected")
        .header("x-gateway-request-from", "app-gateway")
        .header("x-gateway-proxy-request", "true")
        .header("x-gateway-app-id", "app_aion")
        .header("x-gateway-user-id", "external_u1")
        .header("x-gateway-timestamp", timestamp)
        .header("x-gateway-user-id-signature", signature)
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .body(Body::empty())
        .unwrap()
}

fn current_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()
}

struct FakeLegacyUserRepo;

#[async_trait::async_trait]
impl IUserRepository for FakeLegacyUserRepo {
    async fn has_users(&self) -> Result<bool, DbError> {
        Ok(true)
    }

    async fn get_system_user(&self) -> Result<Option<aionui_db::models::User>, DbError> {
        Ok(None)
    }

    async fn get_primary_webui_user(&self) -> Result<Option<aionui_db::models::User>, DbError> {
        Ok(None)
    }

    async fn set_system_user_credentials(&self, _username: &str, _password_hash: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn create_user(&self, _username: &str, _password_hash: &str) -> Result<aionui_db::models::User, DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn ensure_user_with_id(&self, _id: &str, _username: &str) -> Result<aionui_db::models::User, DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn find_by_username(&self, _username: &str) -> Result<Option<aionui_db::models::User>, DbError> {
        Ok(None)
    }

    async fn find_by_id(&self, _id: &str) -> Result<Option<aionui_db::models::User>, DbError> {
        Ok(None)
    }

    async fn list_users(&self) -> Result<Vec<aionui_db::models::User>, DbError> {
        Ok(Vec::new())
    }

    async fn count_users(&self) -> Result<i64, DbError> {
        Ok(0)
    }

    async fn update_password(&self, _user_id: &str, _password_hash: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_username(&self, _user_id: &str, _username: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_last_login(&self, _user_id: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_jwt_secret(&self, _user_id: &str, _jwt_secret: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }
}

struct FakePlatformUserRepo {
    user: Option<PlatformUserRow>,
}

#[async_trait::async_trait]
impl IPlatformUserRepository for FakePlatformUserRepo {
    async fn create_user(&self, _params: CreatePlatformUserParams) -> Result<PlatformUserRow, DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn find_by_id(&self, user_id: &str) -> Result<Option<PlatformUserRow>, DbError> {
        Ok(self.user.clone().filter(|user| user.id == user_id))
    }

    async fn find_by_phone(&self, _phone: &str) -> Result<Option<PlatformUserRow>, DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_from_sync(
        &self,
        _user_id: &str,
        _phone: Option<&str>,
        _display_name: Option<&str>,
        _email: Option<&str>,
        _status: &str,
    ) -> Result<PlatformUserRow, DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_status(&self, _user_id: &str, _status: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_password_hash(&self, _user_id: &str, _password_hash: Option<&str>) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn update_last_login(&self, _user_id: &str) -> Result<(), DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn list_users(&self, _limit: i64, _offset: i64) -> Result<Vec<PlatformUserRow>, DbError> {
        unreachable!("not used by gateway middleware tests")
    }
}

struct FakeExternalIdentityRepo {
    identity: Option<ExternalIdentityRow>,
}

#[async_trait::async_trait]
impl IExternalIdentityRepository for FakeExternalIdentityRepo {
    async fn upsert_identity(&self, _params: UpsertExternalIdentityParams) -> Result<ExternalIdentityRow, DbError> {
        unreachable!("not used by gateway middleware tests")
    }

    async fn find_identity(
        &self,
        provider: &str,
        app_id: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalIdentityRow>, DbError> {
        Ok(self.identity.clone().filter(|identity| {
            identity.provider == provider && identity.app_id == app_id && identity.external_user_id == external_user_id
        }))
    }

    async fn list_by_user(&self, _user_id: &str) -> Result<Vec<ExternalIdentityRow>, DbError> {
        unreachable!("not used by gateway middleware tests")
    }
}

fn platform_user(status: &str) -> PlatformUserRow {
    PlatformUserRow {
        id: "user_1".into(),
        phone: Some("13800138000".into()),
        username: None,
        display_name: Some("User One".into()),
        email: None,
        password_hash: None,
        avatar_path: None,
        status: status.into(),
        jwt_secret: None,
        created_at: 1,
        updated_at: 1,
        last_login: None,
    }
}

fn external_identity() -> ExternalIdentityRow {
    ExternalIdentityRow {
        id: "ext_1".into(),
        user_id: "user_1".into(),
        provider: "modo_open_platform".into(),
        app_id: "app_aion".into(),
        external_user_id: "external_u1".into(),
        phone_snapshot: Some("13800138000".into()),
        external_role_ids: serde_json::json!([]),
        is_admin: false,
        raw_payload: serde_json::json!({}),
        last_synced_at: 1,
        created_at: 1,
        updated_at: 1,
    }
}

fn expired_token(jwt_service: &JwtService, secret: &str, user_id: &str, username: &str) -> String {
    let token = jwt_service.sign(user_id, username).unwrap();
    let mut validation = Validation::default();
    validation.validate_exp = false;
    validation.validate_aud = false;

    let mut claims = decode::<TokenPayload>(&token, &DecodingKey::from_secret(secret.as_bytes()), &validation)
        .unwrap()
        .claims;

    claims.iat = 1000;
    claims.exp = 1001;

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

#[tokio::test]
async fn auth_middleware_missing_token_returns_unauthorized_code() {
    let app = auth_app(Arc::new(JwtService::new("middleware_test_secret".into()))).await;

    let resp = app
        .oneshot(Request::get("/protected").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 401);
}

#[tokio::test]
async fn auth_middleware_invalid_token_returns_unauthorized_code() {
    let app = auth_app(Arc::new(JwtService::new("middleware_test_secret".into()))).await;

    let resp = app
        .oneshot(
            Request::get("/protected")
                .header(header::AUTHORIZATION, "Bearer not-a-valid-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 401);
}

#[tokio::test]
async fn gateway_auth_valid_headers_inject_current_user() {
    let app = gateway_auth_app(
        Arc::new(FakePlatformUserRepo {
            user: Some(platform_user("enabled")),
        }),
        Arc::new(FakeExternalIdentityRepo {
            identity: Some(external_identity()),
        }),
        300,
    );
    let timestamp = current_timestamp();
    let signature = gateway_user_signature("app_aion", "external_u1", &timestamp, "secret_123");

    let resp = app
        .oneshot(gateway_request(&signature, "secret_123", &timestamp))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "user_1:13800138000");
}

#[tokio::test]
async fn gateway_auth_rejects_invalid_app_secret() {
    let app = gateway_auth_app(
        Arc::new(FakePlatformUserRepo {
            user: Some(platform_user("enabled")),
        }),
        Arc::new(FakeExternalIdentityRepo {
            identity: Some(external_identity()),
        }),
        300,
    );
    let timestamp = current_timestamp();
    let signature = gateway_user_signature("app_aion", "external_u1", &timestamp, "secret_123");

    let resp = app
        .oneshot(gateway_request(&signature, "wrong_secret", &timestamp))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_auth_rejects_invalid_signature() {
    let app = gateway_auth_app(
        Arc::new(FakePlatformUserRepo {
            user: Some(platform_user("enabled")),
        }),
        Arc::new(FakeExternalIdentityRepo {
            identity: Some(external_identity()),
        }),
        300,
    );
    let timestamp = current_timestamp();

    let resp = app
        .oneshot(gateway_request("bad_signature", "secret_123", &timestamp))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_auth_rejects_expired_timestamp() {
    let app = gateway_auth_app(
        Arc::new(FakePlatformUserRepo {
            user: Some(platform_user("enabled")),
        }),
        Arc::new(FakeExternalIdentityRepo {
            identity: Some(external_identity()),
        }),
        1,
    );
    let timestamp = "1";
    let signature = gateway_user_signature("app_aion", "external_u1", timestamp, "secret_123");

    let resp = app
        .oneshot(gateway_request(&signature, "secret_123", timestamp))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_auth_rejects_unsynced_user() {
    let app = gateway_auth_app(
        Arc::new(FakePlatformUserRepo {
            user: Some(platform_user("enabled")),
        }),
        Arc::new(FakeExternalIdentityRepo { identity: None }),
        300,
    );
    let timestamp = current_timestamp();
    let signature = gateway_user_signature("app_aion", "external_u1", &timestamp, "secret_123");

    let resp = app
        .oneshot(gateway_request(&signature, "secret_123", &timestamp))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_auth_rejects_disabled_user() {
    let app = gateway_auth_app(
        Arc::new(FakePlatformUserRepo {
            user: Some(platform_user("disabled")),
        }),
        Arc::new(FakeExternalIdentityRepo {
            identity: Some(external_identity()),
        }),
        300,
    );
    let timestamp = current_timestamp();
    let signature = gateway_user_signature("app_aion", "external_u1", &timestamp, "secret_123");

    let resp = app
        .oneshot(gateway_request(&signature, "secret_123", &timestamp))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_middleware_expired_token_returns_unauthorized_code() {
    let secret = "middleware_test_secret";
    let jwt_service = Arc::new(JwtService::new(secret.into()));
    let token = expired_token(&jwt_service, secret, "system_default_user", "system_default_user");
    let app = auth_app(jwt_service).await;

    let resp = app
        .oneshot(
            Request::get("/protected")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 401);
}

#[tokio::test]
async fn auth_middleware_missing_user_returns_unauthorized_code() {
    let jwt_service = Arc::new(JwtService::new("middleware_test_secret".into()));
    let token = jwt_service.sign("missing_user", "ghost").unwrap();
    let app = auth_app(jwt_service).await;

    let resp = app
        .oneshot(
            Request::get("/protected")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 401);
}

#[tokio::test]
async fn auth_middleware_database_error_returns_internal_error_code() {
    let jwt_service = Arc::new(JwtService::new("middleware_test_secret".into()));
    let token = jwt_service.sign("system_default_user", "system_default_user").unwrap();
    let db = init_database_memory().await.unwrap();
    let user_repo = Arc::new(SqliteUserRepository::new(db.pool().clone())) as Arc<dyn IUserRepository>;
    db.close().await;
    let app = protected_auth_app(jwt_service, user_repo);

    let resp = app
        .oneshot(
            Request::get("/protected")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 500);
    assert_eq!(json["message"], "Internal server error.");
    let error = json["message"].as_str().unwrap();
    assert!(!error.contains("Database error"));
    assert!(!error.contains("Authentication service unavailable"));
    assert!(!error.to_ascii_lowercase().contains("closed"));
    assert!(!error.to_ascii_lowercase().contains("sqlx"));
}

#[tokio::test]
async fn t12_2_login_exempt_from_csrf() {
    let app = csrf_app();
    let resp = app
        .oneshot(Request::post("/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn t12_2_qr_login_exempt_from_csrf() {
    let app = csrf_app();
    let resp = app
        .oneshot(Request::post("/api/auth/qr-login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn t12_2_csrf_cookie_set_on_first_request() {
    let app = csrf_app();
    let resp = app
        .oneshot(Request::get("/get-test").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let set_cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie.contains("aionui-csrf-token="));
    // NOT HttpOnly (JS must read it)
    assert!(!set_cookie.contains("HttpOnly"));
}

// ============================================================
// Rate limiter middleware
// ============================================================

fn rate_limit_app(limiter: Arc<RateLimiter>) -> Router {
    Router::new()
        .route("/test", get(|| async { "ok" }))
        .layer(middleware::from_fn_with_state(limiter, api_rate_limit_middleware))
}

#[tokio::test]
async fn api_rate_limit_allows_within_quota() {
    let limiter = Arc::new(RateLimiter::new(3, Duration::from_secs(60)));
    let app = rate_limit_app(limiter);

    for _ in 0..3 {
        let resp = app
            .clone()
            .oneshot(Request::get("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn api_rate_limit_rejects_over_quota() {
    let limiter = Arc::new(RateLimiter::new(2, Duration::from_secs(60)));
    let app = rate_limit_app(limiter);

    // First two pass
    for _ in 0..2 {
        let resp = app
            .clone()
            .oneshot(Request::get("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Third rejected
    let resp = app
        .oneshot(Request::get("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn auth_rate_limit_skips_successful_responses() {
    let limiter = Arc::new(RateLimiter::new(2, Duration::from_secs(60)));
    let app = Router::new()
        .route("/login", post(|| async { "ok" }))
        .layer(middleware::from_fn_with_state(limiter, auth_rate_limit_middleware));

    // Successful responses (200) don't count toward the limit
    for _ in 0..5 {
        let resp = app
            .clone()
            .oneshot(Request::post("/login").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn auth_rate_limit_counts_failed_responses() {
    let limiter = Arc::new(RateLimiter::new(2, Duration::from_secs(60)));
    let app = Router::new()
        .route("/login", post(|| async { StatusCode::UNAUTHORIZED }))
        .layer(middleware::from_fn_with_state(limiter, auth_rate_limit_middleware));

    // First two failures pass through
    for _ in 0..2 {
        let resp = app
            .clone()
            .oneshot(Request::post("/login").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // Third request blocked by rate limiter
    let resp = app
        .oneshot(Request::post("/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn authenticated_action_limit_uses_user_id_key() {
    let limiter = Arc::new(RateLimiter::new(1, Duration::from_secs(60)));

    // Handler that injects a CurrentUser extension before the limiter
    let app = Router::new()
        .route("/action", post(|| async { "done" }))
        .layer(middleware::from_fn_with_state(
            limiter.clone(),
            authenticated_action_rate_limit_middleware,
        ))
        .layer(middleware::from_fn(
            |mut request: axum::extract::Request, next: axum::middleware::Next| async {
                request.extensions_mut().insert(CurrentUser {
                    id: "user_42".into(),
                    username: "admin".into(),
                });
                Ok::<_, std::convert::Infallible>(next.run(request).await)
            },
        ));

    // First request passes
    let resp = app
        .clone()
        .oneshot(Request::post("/action").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Second request for same user is rate limited
    let resp = app
        .oneshot(Request::post("/action").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

// ============================================================
// T12.3 — Cookie security attributes (via CookieConfig)
// ============================================================

#[test]
fn t12_3_session_cookie_is_httponly() {
    let config = CookieConfig {
        secure: false,
        same_site: "Lax",
    };
    let cookie = config.build_session_cookie("token123");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Max-Age="));
}

#[test]
fn t12_3_session_cookie_secure_when_https() {
    let config = CookieConfig {
        secure: true,
        same_site: "Strict",
    };
    let cookie = config.build_session_cookie("token123");
    assert!(cookie.contains("; Secure"));
    assert!(cookie.contains("SameSite=Strict"));
}

// ============================================================
// T13 — Token extraction strategy
// ============================================================

#[test]
fn t13_1_authorization_header_takes_priority() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::AUTHORIZATION, "Bearer header_tok".parse().unwrap());
    headers.insert(header::COOKIE, "aionui-session=cookie_tok".parse().unwrap());
    assert_eq!(
        aionui_auth::extract_token_from_headers(&headers),
        Some("header_tok".into())
    );
}

#[test]
fn t13_2_cookie_fallback() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::COOKIE, "aionui-session=fallback_tok".parse().unwrap());
    assert_eq!(
        aionui_auth::extract_token_from_headers(&headers),
        Some("fallback_tok".into())
    );
}

#[test]
fn t13_3_no_token_returns_none() {
    let headers = axum::http::HeaderMap::new();
    assert_eq!(aionui_auth::extract_token_from_headers(&headers), None);
}
