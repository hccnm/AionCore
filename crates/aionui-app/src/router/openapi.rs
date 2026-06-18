//! OpenAPI spec generation and Swagger UI serving.
//!
//! Uses `utoipa` to generate an OpenAPI 3.0 spec from annotated handler
//! functions and type schemas. The spec is served as JSON at
//! `/api-docs/openapi.json` and rendered as an interactive Swagger UI
//! at `/docs`.

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "AionCore API",
        description = "AionCore backend HTTP API for SaaS remote deployment.",
        version = "0.1.28",
        license(name = "MIT")
    ),
    paths(
        // Health
        super::health::health_check,
        // Auth
        aionui_auth::routes::login_handler,
        aionui_auth::routes::logout_handler,
        aionui_auth::routes::status_handler,
        aionui_auth::routes::setup_password_handler,
        aionui_auth::routes::user_handler,
        aionui_auth::routes::change_password_handler,
        aionui_auth::routes::refresh_handler,
        aionui_auth::routes::ws_token_handler,
        // System
        aionui_system::routes::get_settings,
        aionui_system::routes::update_settings,
        aionui_system::routes::get_system_info,
        aionui_system::routes::list_providers,
        aionui_system::routes::create_provider,
        aionui_system::routes::update_provider,
        aionui_system::routes::delete_provider,
        // Conversation
        aionui_conversation::routes::create,
        aionui_conversation::routes::list,
        aionui_conversation::routes::get_one,
        aionui_conversation::routes::update,
        aionui_conversation::routes::delete_one,
        aionui_conversation::routes::list_msg,
        aionui_conversation::routes::send_msg,
        aionui_conversation::routes::cancel,
        aionui_conversation::routes::active_count,
    ),
    components(schemas(
        // Common
        aionui_api_types::ErrorResponse,
        // Auth
        aionui_api_types::LoginRequest,
        aionui_api_types::LoginResponse,
        aionui_api_types::PublicUser,
        aionui_api_types::AuthStatusResponse,
        aionui_api_types::ChangePasswordRequest,
        aionui_api_types::WebuiChangePasswordRequest,
        aionui_api_types::RefreshTokenRequest,
        aionui_api_types::RefreshResponse,
        aionui_api_types::UserInfoResponse,
        aionui_api_types::WsTokenResponse,
        // System
        aionui_api_types::SystemSettingsResponse,
        aionui_api_types::UpdateSettingsRequest,
        aionui_api_types::SystemInfoResponse,
        aionui_api_types::ProviderResponse,
        aionui_api_types::CreateProviderRequest,
        aionui_api_types::UpdateProviderRequest,
        // Conversation
        aionui_api_types::CreateConversationRequest,
        aionui_api_types::UpdateConversationRequest,
        aionui_api_types::ConversationResponse,
        aionui_api_types::SendMessageRequest,
        aionui_api_types::SendMessageResponse,
        aionui_api_types::MessageResponse,
        aionui_api_types::CancelConversationRequest,
        aionui_api_types::ConversationRuntimeSummary,
        aionui_api_types::ActiveCountResponse,
    )),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "auth", description = "Authentication & authorization"),
        (name = "system", description = "System settings, info & providers"),
        (name = "conversation", description = "Conversation & message management")
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Add Bearer auth security scheme to the OpenAPI spec.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

        let components = openapi.components.get_or_insert_default();
        components.security_schemes.insert(
            "bearer_auth".to_string(),
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

/// Build the router serving Swagger UI and OpenAPI JSON.
pub fn openapi_routes() -> axum::Router {
    let openapi = ApiDoc::openapi();

    utoipa_swagger_ui::SwaggerUi::new("/docs")
        .url("/api-docs/openapi.json", openapi)
        .into()
}
