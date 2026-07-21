// Auth & CORS hardening for the SSE (HTTP) transport.
//
// The stdio transport is unaffected — these helpers only wrap the axum router
// used by `--transport sse`. Design (issue M0-2):
//   * bind loopback by default (enforced in main.rs)
//   * require a bearer token to expose over the network
//   * restrict CORS instead of allowing any origin
//
// `bearer_ok` and `is_loopback` are pure and unit-tested below; `require_bearer`
// is the axum middleware; `cors_layer` builds the CORS policy from --cors-origin.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;
use tower_http::cors::{Any, AllowOrigin, CorsLayer};

/// True if `host` refers to the local machine (loopback), so it is safe to
/// serve without authentication. `localhost` plus any loopback IP (127.0.0.0/8,
/// ::1) qualify; everything else is treated as network-exposed.
pub fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Validate an `Authorization: Bearer <token>` header against `expected` in
/// constant time. Returns false when the header is missing, malformed, or the
/// token does not match. Constant-time comparison avoids leaking the token
/// through response timing.
pub fn bearer_ok(headers: &HeaderMap, expected: &[u8]) -> bool {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = value.to_str() else {
        return false;
    };
    let Some(token) = s.strip_prefix("Bearer ") else {
        return false;
    };
    // subtle's ct_eq over slices returns 0 for unequal lengths without an early
    // return that would leak the token length beyond what the caller controls.
    token.trim().as_bytes().ct_eq(expected).into()
}

/// Axum middleware: reject any request whose bearer token does not match the
/// configured one with `401 Unauthorized`. Wired only when a token is present.
pub async fn require_bearer(
    State(token): State<Arc<String>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if bearer_ok(req.headers(), token.as_bytes()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Build the CORS policy from `--cors-origin` values:
///   * empty        → `None` (no CORS layer; same-origin only)
///   * contains `*` → allow any origin (explicit opt-in)
///   * otherwise    → allow exactly the listed origins
pub fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    if origins.iter().any(|o| o == "*") {
        return Some(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    }
    let list: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| o.parse::<HeaderValue>().ok())
        .collect();
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(list))
            .allow_methods(Any)
            .allow_headers(Any),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};

    fn headers_with(auth: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_str(auth).unwrap());
        h
    }

    #[test]
    fn bearer_missing_header_is_rejected() {
        assert!(!bearer_ok(&HeaderMap::new(), b"secret"));
    }

    #[test]
    fn bearer_wrong_token_is_rejected() {
        assert!(!bearer_ok(&headers_with("Bearer nope"), b"secret"));
    }

    #[test]
    fn bearer_missing_prefix_is_rejected() {
        assert!(!bearer_ok(&headers_with("secret"), b"secret"));
    }

    #[test]
    fn bearer_correct_token_is_accepted() {
        assert!(bearer_ok(&headers_with("Bearer secret"), b"secret"));
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("localhost"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.168.1.10"));
    }

    #[test]
    fn cors_none_when_empty() {
        assert!(cors_layer(&[]).is_none());
    }

    #[test]
    fn cors_some_when_configured() {
        assert!(cors_layer(&["https://example.com".to_string()]).is_some());
        assert!(cors_layer(&["*".to_string()]).is_some());
    }

    // End-to-end: a router guarded by require_bearer answers 401 without the
    // token and 200 with it. Driven in-memory via `oneshot` — no HTTP client.
    #[tokio::test]
    async fn auth_middleware_gates_requests() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt; // for `oneshot`

        let token = Arc::new("s3cret".to_string());
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                token.clone(),
                require_bearer,
            ));

        let send = |auth: Option<&'static str>| {
            let app = app.clone();
            async move {
                let mut req = Request::builder().uri("/");
                if let Some(a) = auth {
                    req = req.header(AUTHORIZATION, a);
                }
                app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
            }
        };

        assert_eq!(send(None).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            send(Some("Bearer wrong")).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(send(Some("Bearer s3cret")).await.status(), StatusCode::OK);
    }
}
