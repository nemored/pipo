use core::fmt;
use std::marker::PhantomData;

use axum::{
    body::Body,
    http::{header, HeaderValue, Request, StatusCode, Uri},
    response::Response,
    routing::{get, put, post},
    Router,
    extract::Path,
};
use bytes::BytesMut;
use ruma::api::{
    client::error::{ErrorBody, ErrorKind},
    OutgoingResponse,
};
use tower_http::validate_request::{ValidateRequest, ValidateRequestHeaderLayer};

enum MatrixErrorCode {
    MForbidden,
}

struct MatrixError {
    errcode: MatrixErrorCode,
    error: String,
}

struct MatrixBearer {
    header_value: HeaderValue,
    _ty: PhantomData<fn() -> Body>,
}

impl MatrixBearer {
    fn new(token: &str) -> Self {
        Self {
            header_value: format!("Bearer {}", token)
                .parse()
                .expect("token is not a valid header value"),
            _ty: PhantomData,
        }
    }
}

impl Clone for MatrixBearer {
    fn clone(&self) -> Self {
        Self {
            header_value: self.header_value.clone(),
            _ty: PhantomData,
        }
    }
}

impl fmt::Debug for MatrixBearer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bearer")
            .field("header_value", &self.header_value)
            .finish()
    }
}

impl<B> ValidateRequest<B> for MatrixBearer {
    type ResponseBody = Body;

    fn validate(&mut self, request: &mut Request<B>) -> Result<(), Response<Self::ResponseBody>> {
        match request.headers().get(header::AUTHORIZATION) {
            Some(actual) if actual == self.header_value => Ok(()),
            _ => Err(ErrorBody::Standard {
                kind: ErrorKind::Forbidden,
                message: "Bad token supplied".to_string(),
            }
            .into_error(StatusCode::FORBIDDEN)
            .try_into_http_response::<BytesMut>()
            .expect("failed to construct response")
            .map(|b| b.freeze().into())
            .into()),
        }
    }
}

struct Http {
    app: Router,
}

impl Http {
    fn new() -> Self {
        Self { app: Router::new() }
    }
    pub fn add_matrix_route(&mut self, hs_token: &str) {
        self.app = self
            .app
            .clone()
            .route("/_matrix/", get(|| async {})) .fallback(unsupported_method)// TODO: request method
            .route("/_matrix/app/v1/users/:userId", put(get_user)).fallback(unsupported_method)
            .route("/_matrix/app/v1/transactions/:txnId", put(get_transaction)).fallback(unsupported_method)
            .route("/_matrix/app/v1/rooms/:roomAlias", get(get_room)).fallback(unsupported_method)
            .route("/_matrix/app/v1/thirdparty/protocol/:protocol", get(get_thirdparty_protocol)).fallback(unsupported_method)
            .route("/_matrix/app/v1/ping", post(handle_ping)).fallback(unsupported_method)
            .route("/_matrix/app/v1/thirdparty/location", get(get_location)).fallback(unsupported_method)
            .route("/_matrix/app/v1/thirdparty/location/:protocol", get(get_location_protocol)).fallback(unsupported_method)
            .route("/_matrix/app/v1/thirdparty/user", get(get_thirdparty_user)).fallback(unsupported_method)
            .route("/_matrix/app/v1/thirdparty/user/:protocol", get(get_thirdparty_user)).fallback(unsupported_method)
            .fallback(unsupported_method)
            .fallback(unknown_route)
            .route_layer(ValidateRequestHeaderLayer::custom(MatrixBearer::new(
                hs_token,
            )));
    }
    // pub async fn serve(&self, listener: TcpListener) {
    //     axum::serve(listener, self.app).await.unwrap();
    // }
}


async fn get_thirdparty_user_protocol() {
    todo!("get tp user protocol")
}

async fn get_thirdparty_user() {
    todo!("get tp user")
}

async fn get_location_protocol() {
    todo!("get loc protocol")
}

async fn get_location() {
    todo!("get location")
}

async fn handle_ping() {
    todo!("ping")
}

async fn get_thirdparty_protocol(Path(protocol): Path<String>) -> String {
    todo!("protocol")
}


async fn get_room(Path(room): Path<String>) -> String {
    todo!("room")
}

async fn get_transaction(Path(tid): Path<String>) -> String {
    todo!("txn")
}

async fn get_user(Path(user_id): Path<String>) -> String {
    todo!("get user")
}


async fn unknown_route(uri: Uri) -> Response {
    ErrorBody::Standard {
        kind: ErrorKind::Unrecognized,
        message: "Unknown endpoint".to_string(),
    }
    .into_error(StatusCode::NOT_FOUND)
    .try_into_http_response::<BytesMut>()
    .expect("failed to construct response")
    .map(|b| b.freeze().into())
    .into()
}

async fn unsupported_method(uri: Uri) -> Response {
    ErrorBody::Standard {
        kind: ErrorKind::Unrecognized,
        message: "Unsupported method".to_string(),
    }
    .into_error(StatusCode::METHOD_NOT_ALLOWED)
    .try_into_http_response::<BytesMut>()
    .expect("failed to construct response")
    .map(|b| b.freeze().into())
    .into()
}

#[cfg(test)]
mod tests {
    use axum::{body::HttpBody, response::Json};
    use serde_json::{json, Value};
    use tower_service::Service;

    use super::*;

    const TEST_TOKEN: &'static str = "unit-test";

    async fn test_response(hs_token: &str, request: Request<Body>, expected: Response) {
        let mut http = Http::new();
        http.add_matrix_route(hs_token);
        // TODO(nemo): Decide if using tower_service::oneshot() is better
        // than call().
        let res = http.app.call(request).await.unwrap();
        assert_eq!(res.status(), expected.status());
        let res_body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let exp_body = axum::body::to_bytes(expected.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(res_body, exp_body);
    }

    // TODO(nemo): Clean up test
    #[tokio::test]
    async fn matrix_handle_invalid_token() {
        let hs_token = "abcd";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/")
            .header(header::AUTHORIZATION, "Bearer dcba")
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::new(
                json!({ "errcode": "M_FORBIDDEN", "error": "Bad token supplied" }).to_string(),
            ))
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_valid_token() {
        let hs_token = "abcd";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_unknown_endpoint() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/unknown/")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::new(
                json!({ "errcode": "M_UNRECOGNIZED", "error": "Unknown endpoint" }).to_string(),
            ))
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_known_endpoint() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_unsupported_method() {
        let hs_token = TEST_TOKEN;
        let request = Request::builder()
            .method("POST")
            .uri("/_matrix/")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::new(
                json!({ "errcode": "M_UNRECOGNIZED", "error": "Unsupported method" }).to_string(),
            ))
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_users() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("PUT")
            .uri("/_matrix/app/v1/users/1")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_put_transactions() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("PUT")
            .uri("/_matrix/app/v1/transactions/1")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_room_alias() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/rooms/room-alias")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_protocol() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/thirdparty/protocol/chosen-protocol")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_thirdparty_protocol() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/thirdparty/protocol/chosen-protocol")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_post_ping() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("POST")
            .uri("/_matrix/app/v1/ping")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_thirdparty_location() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/thirdparty/location")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_thirdparty_location_protocol() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/thirdparty/location/chosen-protocol")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_thirdparty_user() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/thirdparty/user")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }

    #[tokio::test]
    async fn matrix_handle_get_thirdparty_user_protocol() {
        let hs_token = "test_handle_unknown_endpoint";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/app/v1/thirdparty/user/chosen-user-protocol")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::empty())
            .unwrap();
        test_response(hs_token, request, expected).await;
    }
}
