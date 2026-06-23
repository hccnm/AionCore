use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tokio::sync::RwLock;
use tower::ServiceExt;

use aionui_auth::{AuthRouterState, CookieConfig, JwtService, QrTokenStore, auth_routes, hash_password};
use aionui_db::{
    CreatePlatformUserParams, DbError, ExternalIdentityRow, IExternalIdentityRepository, IPlatformUserRepository,
    IRoleRepository, IUserRepository, PlatformUserRow, RoleRow, SqliteUserRepository, UpsertExternalIdentityParams,
    UpsertRoleParams, init_database_memory,
};

#[derive(Default)]
struct FakePlatformUserRepo {
    user: RwLock<Option<PlatformUserRow>>,
    last_login_updates: RwLock<Vec<String>>,
}

#[async_trait::async_trait]
impl IPlatformUserRepository for FakePlatformUserRepo {
    async fn create_user(&self, _params: CreatePlatformUserParams) -> Result<PlatformUserRow, DbError> {
        unreachable!("not needed")
    }

    async fn find_by_id(&self, user_id: &str) -> Result<Option<PlatformUserRow>, DbError> {
        Ok(self.user.read().await.clone().filter(|user| user.id == user_id))
    }

    async fn find_by_phone(&self, phone: &str) -> Result<Option<PlatformUserRow>, DbError> {
        Ok(self
            .user
            .read()
            .await
            .clone()
            .filter(|user| user.phone.as_deref() == Some(phone)))
    }

    async fn update_from_sync(
        &self,
        _user_id: &str,
        _phone: Option<&str>,
        _display_name: Option<&str>,
        _email: Option<&str>,
        _status: &str,
    ) -> Result<PlatformUserRow, DbError> {
        unreachable!("not needed")
    }

    async fn update_status(&self, _user_id: &str, _status: &str) -> Result<(), DbError> {
        unreachable!("not needed")
    }

    async fn update_password_hash(&self, _user_id: &str, _password_hash: Option<&str>) -> Result<(), DbError> {
        unreachable!("not needed")
    }

    async fn update_last_login(&self, user_id: &str) -> Result<(), DbError> {
        self.last_login_updates.write().await.push(user_id.to_owned());
        Ok(())
    }

    async fn list_users(&self, _limit: i64, _offset: i64) -> Result<Vec<PlatformUserRow>, DbError> {
        Ok(self.user.read().await.iter().cloned().collect())
    }
}

#[derive(Default)]
struct FakeExternalIdentityRepo;

#[async_trait::async_trait]
impl IExternalIdentityRepository for FakeExternalIdentityRepo {
    async fn upsert_identity(&self, _params: UpsertExternalIdentityParams) -> Result<ExternalIdentityRow, DbError> {
        unreachable!("not needed")
    }

    async fn find_identity(
        &self,
        _provider: &str,
        _app_id: &str,
        _external_user_id: &str,
    ) -> Result<Option<ExternalIdentityRow>, DbError> {
        Ok(None)
    }

    async fn list_by_user(&self, _user_id: &str) -> Result<Vec<ExternalIdentityRow>, DbError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct FakeRoleRepo {
    roles: RwLock<Vec<RoleRow>>,
}

#[async_trait::async_trait]
impl IRoleRepository for FakeRoleRepo {
    async fn upsert_role(&self, _params: UpsertRoleParams) -> Result<RoleRow, DbError> {
        unreachable!("not needed")
    }

    async fn list_roles(&self) -> Result<Vec<RoleRow>, DbError> {
        Ok(self.roles.read().await.clone())
    }

    async fn set_role_status(&self, _role_id: &str, _status: &str) -> Result<(), DbError> {
        unreachable!("not needed")
    }

    async fn assign_role(&self, _user_id: &str, _role_id: &str) -> Result<(), DbError> {
        unreachable!("not needed")
    }

    async fn revoke_role(&self, _user_id: &str, _role_id: &str) -> Result<(), DbError> {
        unreachable!("not needed")
    }

    async fn list_user_roles(&self, _user_id: &str) -> Result<Vec<RoleRow>, DbError> {
        Ok(self.roles.read().await.clone())
    }
}

async fn test_state(
    platform_user_repo: Arc<FakePlatformUserRepo>,
    role_repo: Arc<FakeRoleRepo>,
) -> (axum::Router, Arc<JwtService>) {
    let db = init_database_memory().await.unwrap();
    let user_repo = Arc::new(SqliteUserRepository::new(db.pool().clone())) as Arc<dyn IUserRepository>;
    let jwt_service = Arc::new(JwtService::new("workbench_route_secret".into()));
    let state = AuthRouterState {
        jwt_service: jwt_service.clone(),
        user_repo,
        platform_user_repo: Some(platform_user_repo),
        external_identity_repo: Some(Arc::new(FakeExternalIdentityRepo)),
        role_repo: Some(role_repo),
        gateway_auth: None,
        cookie_config: Arc::new(CookieConfig {
            secure: false,
            same_site: "Lax",
        }),
        qr_token_store: Arc::new(QrTokenStore::new()),
        local: false,
    };
    (auth_routes(state), jwt_service)
}

fn platform_user(password_hash: Option<String>) -> PlatformUserRow {
    platform_user_with_status(password_hash, "enabled")
}

fn platform_user_with_status(password_hash: Option<String>, status: &str) -> PlatformUserRow {
    PlatformUserRow {
        id: "user_platform".into(),
        phone: Some("13800138000".into()),
        username: None,
        display_name: Some("Platform User".into()),
        email: None,
        password_hash,
        avatar_path: None,
        status: status.into(),
        jwt_secret: None,
        created_at: 1,
        updated_at: 1,
        last_login: None,
    }
}

fn role(role_key: &str, status: &str, permissions: serde_json::Value) -> RoleRow {
    RoleRow {
        id: format!("role_{role_key}"),
        role_key: role_key.into(),
        role_name: role_key.into(),
        status: status.into(),
        permissions,
        sort_order: 1,
        is_system: true,
        created_at: 1,
        updated_at: 1,
    }
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn auth_status_uses_platform_users_when_configured() {
    let platform_repo = Arc::new(FakePlatformUserRepo::default());
    *platform_repo.user.write().await = Some(platform_user(None));
    let role_repo = Arc::new(FakeRoleRepo::default());
    let (app, _jwt_service) = test_state(platform_repo, role_repo).await;

    let resp = app
        .oneshot(Request::get("/api/auth/status").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["needs_setup"], false);
    assert_eq!(json["data"]["user_count"], 1);
}

#[tokio::test]
async fn platform_phone_login_accepts_api_auth_login_alias() {
    let password_hash = hash_password("StrongP@ss1").unwrap();
    let platform_repo = Arc::new(FakePlatformUserRepo::default());
    *platform_repo.user.write().await = Some(platform_user(Some(password_hash)));
    let role_repo = Arc::new(FakeRoleRepo::default());
    let (app, _jwt_service) = test_state(platform_repo.clone(), role_repo).await;

    let resp = app
        .oneshot(
            Request::post("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"phone":"13800138000","password":"StrongP@ss1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["user"]["id"], "user_platform");
    assert!(json["data"]["token"].is_string());
    assert_eq!(
        &*platform_repo.last_login_updates.read().await,
        &["user_platform".to_owned()]
    );
}

#[tokio::test]
async fn platform_phone_login_rejects_disabled_user() {
    let password_hash = hash_password("StrongP@ss1").unwrap();
    let platform_repo = Arc::new(FakePlatformUserRepo::default());
    *platform_repo.user.write().await = Some(platform_user_with_status(Some(password_hash), "disabled"));
    let role_repo = Arc::new(FakeRoleRepo::default());
    let (app, _jwt_service) = test_state(platform_repo.clone(), role_repo).await;

    let resp = app
        .oneshot(
            Request::post("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"phone":"13800138000","password":"StrongP@ss1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(platform_repo.last_login_updates.read().await.is_empty());
}

#[tokio::test]
async fn current_user_ignores_disabled_admin_roles() {
    let platform_repo = Arc::new(FakePlatformUserRepo::default());
    *platform_repo.user.write().await = Some(platform_user(None));
    let role_repo = Arc::new(FakeRoleRepo::default());
    *role_repo.roles.write().await = vec![
        role("super_admin", "disabled", serde_json::json!(["*"])),
        role("ordinary_user", "enabled", serde_json::json!(["workspace:own"])),
    ];
    let (app, jwt_service) = test_state(platform_repo, role_repo).await;
    let token = jwt_service.sign("user_platform", "13800138000").unwrap();

    let resp = app
        .oneshot(
            Request::get("/api/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["data"]["is_admin"], false);
    assert_eq!(json["data"]["permission_flags"], serde_json::json!(["workspace:own"]));
    assert_eq!(json["data"]["roles"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn ws_token_accepts_platform_user_token() {
    let platform_repo = Arc::new(FakePlatformUserRepo::default());
    *platform_repo.user.write().await = Some(platform_user(None));
    let role_repo = Arc::new(FakeRoleRepo::default());
    let (app, jwt_service) = test_state(platform_repo, role_repo).await;
    let token = jwt_service.sign("user_platform", "13800138000").unwrap();

    let resp = app
        .oneshot(
            Request::get("/api/ws-token")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["ws_token"], token);
}

#[tokio::test]
async fn current_user_reflects_role_assignment_changes() {
    let platform_repo = Arc::new(FakePlatformUserRepo::default());
    *platform_repo.user.write().await = Some(platform_user(None));
    let role_repo = Arc::new(FakeRoleRepo::default());
    *role_repo.roles.write().await = vec![role("ordinary_user", "enabled", serde_json::json!(["workspace:own"]))];
    let (app, jwt_service) = test_state(platform_repo, role_repo.clone()).await;
    let token = jwt_service.sign("user_platform", "13800138000").unwrap();

    let first_resp = app
        .clone()
        .oneshot(
            Request::get("/api/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_resp.status(), StatusCode::OK);
    let first_json = json_body(first_resp).await;
    assert_eq!(first_json["data"]["is_admin"], false);
    assert_eq!(
        first_json["data"]["permission_flags"],
        serde_json::json!(["workspace:own"])
    );

    role_repo
        .roles
        .write()
        .await
        .push(role("super_admin", "enabled", serde_json::json!(["*"])));

    let second_resp = app
        .oneshot(
            Request::get("/api/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_resp.status(), StatusCode::OK);
    let second_json = json_body(second_resp).await;
    assert_eq!(second_json["data"]["is_admin"], true);
    assert_eq!(
        second_json["data"]["permission_flags"],
        serde_json::json!(["*", "workspace:own"])
    );
    assert_eq!(second_json["data"]["roles"].as_array().unwrap().len(), 2);
}
