// SPDX-License-Identifier: FSL-1.1-ALv2
use crate::Server;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{MethodRouter, post},
};
use server_common::http::CorrelationId;
use std::sync::Arc;

/// Uses the same webhook adapter in the API entrypoint and synthetic-provider runtime hosts.
pub fn webhook_route(server: Arc<Server>) -> MethodRouter {
    post(
        move |Extension(correlation): Extension<CorrelationId>,
              headers: HeaderMap,
              body: Result<Bytes, BytesRejection>| {
            let server = server.clone();
            async move {
                let body = match body {
                    Ok(body) => body,
                    Err(rejection) => {
                        tracing::warn!(
                            correlation_id = %correlation.0,
                            category = "malformed",
                            "webhook rejected"
                        );
                        return rejection.into_response();
                    }
                };
                webhook_response(&server, correlation, &headers, &body).await
            }
        },
    )
    .layer(DefaultBodyLimit::max(25 << 20))
}

async fn webhook_response(
    server: &Server,
    correlation: CorrelationId,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let event = headers
        .get("X-GitHub-Event")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    match server.webhook(event, signature, body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(fault) => {
            server.record_failure(&fault);
            let (status, category) = match fault.code {
                connectrpc::ErrorCode::Unauthenticated => (StatusCode::UNAUTHORIZED, "signature"),
                connectrpc::ErrorCode::InvalidArgument => (StatusCode::BAD_REQUEST, "malformed"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            };
            tracing::warn!(
                correlation_id = %correlation.0,
                category,
                "webhook rejected"
            );
            status.into_response()
        }
    }
}
