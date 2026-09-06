// use notify_rust::Notification;
//
// fn main() {
//     Notification::new()
//         .summary("Firefox News")
//         .body("This will almost look like a real firefox notification.")
//         .icon("claude")
//         .show().expect("TODO: panic message");
// }

use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use notify_rust::Notification;
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use thiserror::Error;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use tracing::error;
use tracing_subscriber::EnvFilter;

/// 起動オプション。CLI引数または環境変数で指定できる。
#[derive(Parser, Debug)]
#[command(name = "toster")]
struct Args {
    /// リッスンするポート番号
    #[arg(short, long, default_value_t = 8000, env = "PORT")]
    port: u16,
}

/// アプリケーション全体で扱うエラー型。
/// クライアントには最小限の情報のみ返し、詳細はサーバー側ログに記録する。
#[derive(Debug, Error)]
enum AppError {
    #[error("resource not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, public_message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Internal(err) => {
                // 内部エラーの詳細はログにのみ出力し、クライアントには漏らさない。
                error!(error = ?err, "unhandled internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
        };

        (status, Json(json!({ "error": public_message }))).into_response()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let args = Args::parse();

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| anyhow::anyhow!("failed to bind to {addr}: {err}"))?;

    tracing::info!("listening on {addr}");

    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|err| anyhow::anyhow!("server error: {err}"))?;

    Ok(())
}

fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/notify", post(notify))
        .fallback(not_found)
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn health() -> impl IntoResponse {
    StatusCode::OK
}

async fn not_found() -> AppError {
    AppError::NotFound
}

#[derive(Debug, Deserialize)]
struct NotifyRequest {
    title: String,
    message: String,
}

async fn notify(Json(payload): Json<NotifyRequest>) -> Result<StatusCode, AppError> {
    if payload.title.trim().is_empty() {
        return Err(AppError::BadRequest("title must not be empty".to_string()));
    }

    // notify-rust はブロッキングAPIなので、ランタイムのスレッドを塞がないよう spawn_blocking で実行する。
    tokio::task::spawn_blocking(move || {
        Notification::new()
            .summary(&payload.title)
            .body(&payload.message)
            .show()
    })
    .await
    .map_err(|err| anyhow::anyhow!("notification task panicked: {err}"))?
    .map_err(|err| anyhow::anyhow!("failed to show notification: {err}"))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, starting graceful shutdown");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn json_body(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_returns_200() {
        let response = app()
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_returns_404_with_json_error() {
        let response = app()
            .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = json_body(response).await;
        assert_eq!(json["error"], "resource not found");
    }

    #[tokio::test]
    async fn notify_with_empty_title_returns_400() {
        let request = Request::builder()
            .method("POST")
            .uri("/notify")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"","message":"x"}"#))
            .unwrap();

        let response = app().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(json["error"], "title must not be empty");
    }

    #[tokio::test]
    async fn notify_with_missing_field_returns_422() {
        let request = Request::builder()
            .method("POST")
            .uri("/notify")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"only title"}"#))
            .unwrap();

        let response = app().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    #[ignore = "実際にOSの通知を表示するため、手動で `cargo test -- --ignored` から実行する"]
    async fn notify_with_valid_payload_returns_204() {
        let request = Request::builder()
            .method("POST")
            .uri("/notify")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"test","message":"hello"}"#))
            .unwrap();

        let response = app().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn app_error_not_found_maps_to_404() {
        let response = AppError::NotFound.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn app_error_bad_request_maps_to_400() {
        let response = AppError::BadRequest("bad input".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(json["error"], "bad input");
    }

    #[tokio::test]
    async fn app_error_internal_maps_to_500_and_hides_details() {
        let response = AppError::Internal(anyhow::anyhow!("db exploded")).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let json = json_body(response).await;
        assert_eq!(json["error"], "internal server error");
    }
}
