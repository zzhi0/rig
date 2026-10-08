//! HTTP conversion and static asset delivery.

use std::path::Path;

use axum::{Json, Router, routing::get};
use serde::Serialize;
use tower_http::services::ServeDir;

use crate::application::{ApplicationInfo, application_info};

#[derive(Serialize)]
struct InfoResponse {
    name: &'static str,
    version: &'static str,
}

impl From<ApplicationInfo> for InfoResponse {
    fn from(info: ApplicationInfo) -> Self {
        Self {
            name: info.name,
            version: info.version,
        }
    }
}

/// Builds the local API and file service for the selected asset directory.
pub fn router(web_dir: impl AsRef<Path>) -> Router {
    Router::new()
        .route("/api/info", get(info))
        .fallback_service(ServeDir::new(web_dir))
}

async fn info() -> Json<InfoResponse> {
    Json(application_info().into())
}
