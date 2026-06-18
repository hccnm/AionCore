use std::sync::{Mutex, MutexGuard, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use aionui_app::{AppConfig, AppServices, DeploymentMode};

fn build_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("failed to build request")
}

async fn response_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.expect("failed to read body").to_bytes();
    serde_json::from_slice(&bytes).expect("failed to parse JSON")
}

async fn build_app() -> axum::Router {
    build_app_with_config(AppConfig::default()).await.0
}

async fn build_app_with_config(config: AppConfig) -> (axum::Router, AppServices) {
    let db = aionui_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &config).await.unwrap();
    let router = aionui_app::create_router(&services).await.expect("build router");
    (router, services)
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    name: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let lock = env_lock().lock().unwrap();
        let previous = std::env::var(name).ok();
        // SAFETY: guarded by a test-process mutex and restored on drop.
        unsafe {
            std::env::set_var(name, value);
        }
        Self {
            _lock: lock,
            name,
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: guarded by EnvVarGuard's mutex for this test process.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

#[tokio::test]
async fn health_check_returns_ok() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/health"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response.into_body()).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn healthz_check_returns_ok() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/healthz"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response.into_body()).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn health_check_post_blocked_by_csrf() {
    let app = build_app().await;

    // POST without CSRF token is rejected by the global CSRF middleware
    let response = app
        .oneshot(build_request("POST", "/health"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn saas_mode_post_with_bearer_does_not_require_csrf() {
    let config = AppConfig {
        deployment_mode: DeploymentMode::Saas,
        ..Default::default()
    };
    let (app, services) = build_app_with_config(config).await;
    let token = services.jwt_service.sign("system_default_user", "admin").unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/logout")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn saas_cors_allows_configured_origin_and_headers() {
    let _guard = EnvVarGuard::set("ALLOWED_ORIGINS", "https://app.example.com");
    let config = AppConfig {
        deployment_mode: DeploymentMode::Saas,
        ..Default::default()
    };
    let (app, _services) = build_app_with_config(config).await;

    let request = Request::builder()
        .method("OPTIONS")
        .uri("/api/auth/status")
        .header(header::ORIGIN, "https://app.example.com")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization,x-csrf-token")
        .body(Body::empty())
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "https://app.example.com"
    );
    assert_eq!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            .unwrap(),
        "true"
    );
    let allow_headers = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(allow_headers.contains("authorization"));
    assert!(allow_headers.contains("x-csrf-token"));
}

#[tokio::test]
async fn saas_cors_does_not_allow_unconfigured_origin() {
    let _guard = EnvVarGuard::set("ALLOWED_ORIGINS", "https://app.example.com");
    let config = AppConfig {
        deployment_mode: DeploymentMode::Saas,
        ..Default::default()
    };
    let (app, _services) = build_app_with_config(config).await;

    let request = Request::builder()
        .method("OPTIONS")
        .uri("/api/auth/status")
        .header(header::ORIGIN, "https://evil.example.com")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
        .body(Body::empty())
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");

    assert!(response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn saas_cors_fails_closed_when_no_whitelist() {
    let _guard = EnvVarGuard::set("ALLOWED_ORIGINS", "");
    let config = AppConfig {
        deployment_mode: DeploymentMode::Saas,
        ..Default::default()
    };
    let (app, _services) = build_app_with_config(config).await;

    let request = Request::builder()
        .method("OPTIONS")
        .uri("/api/auth/status")
        .header(header::ORIGIN, "https://any-origin.example.com")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
        .body(Body::empty())
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");

    assert!(response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn saas_mode_does_not_mount_openapi_docs() {
    let config = AppConfig {
        deployment_mode: DeploymentMode::Saas,
        ..Default::default()
    };
    let (app, _services) = build_app_with_config(config).await;

    let response = app
        .oneshot(build_request("GET", "/api-docs/openapi.json"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/nonexistent"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = response_json(response.into_body()).await;
    assert_eq!(json["success"], false);
    assert_eq!(json["code"], "NOT_FOUND");
    assert!(json["error"].is_string());
}

#[tokio::test]
async fn default_body_limit_returns_error_response() {
    let app = build_app().await;

    let body = format!(
        r#"{{"username":"admin","password":"{}"}}"#,
        "x".repeat(11 * 1024 * 1024)
    );
    let request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let json = response_json(response.into_body()).await;
    assert_eq!(json["success"], false);
    assert_eq!(json["code"], "PAYLOAD_TOO_LARGE");
    assert!(json["error"].is_string());
}

#[tokio::test]
async fn health_check_has_security_headers() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/health"))
        .await
        .expect("request failed");

    assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
    assert_eq!(response.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(response.headers().get("x-xss-protection").unwrap(), "1; mode=block");
    assert_eq!(
        response.headers().get("referrer-policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
}
