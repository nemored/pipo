use core::fmt;
use std::marker::PhantomData;

use serde_json::json;
use serde::{Serialize, Deserialize};
use axum::{
    body::Body,
    http::{header, HeaderValue, Request, StatusCode, Uri},
    response::Response,
    routing::{get, put, post},
    Router,
    extract::{Path, Request as RequestExtractor, Query},
};
use bytes::{BytesMut, Bytes};
use ruma::api::{
    client::error::{ErrorBody, ErrorKind},
    OutgoingResponse,
    IncomingRequest,
    OutgoingRequest,
    IncomingResponse,
    appservice::event::push_events::v1::Request as RumaPushEventRequest,
    appservice::thirdparty::get_protocol::v1::Request as RumaGetProtocolRequest,
    appservice::thirdparty::get_user_for_user_id::v1::Request as RumaGetThirdpartyUserForUIDRequest,
    appservice::ping::send_ping::v1::Request as RumaPingRequest,
};
use tower_http::validate_request::{ValidateRequest, ValidateRequestHeaderLayer};

const MATRIX_HANDLERS_RELEASED: bool = false;
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
            .route("/_matrix/app/v1/users/:userId", put(put_user)).fallback(unsupported_method)
            .route("/_matrix/app/v1/transactions/:txnId", put(put_transaction)).fallback(unsupported_method)
            .route("/_matrix/app/v1/rooms/:roomAlias", get(get_room)).fallback(unsupported_method)
            .route("/_matrix/app/v1/thirdparty/protocol/:protocol", get(get_thirdparty_protocol)).fallback(unsupported_method)
            .route("/_matrix/app/v1/ping", post(post_ping)).fallback(unsupported_method)
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

async fn handle_get_thirdparty_user(request: RumaGetThirdpartyUserForUIDRequest) {
    todo!("handle tp get user")
}

#[derive(Deserialize)]
struct GetThirdpartyUser {
    userid: String
}

async fn get_thirdparty_user(userid: Query<GetThirdpartyUser>, request: RequestExtractor) -> Response {
    let req: RumaGetThirdpartyUserForUIDRequest = RumaGetThirdpartyUserForUIDRequest::try_from_http_request(
        into_bytes_request(request).await,
        &vec![userid.userid.to_owned()]
    ).unwrap();

    if MATRIX_HANDLERS_RELEASED {
        // do whatever it takes.
        handle_get_thirdparty_user(req).await;
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::new(json!({}).to_string())).unwrap();

    response
}

async fn get_location_protocol() {
    todo!("get loc protocol")
}

async fn get_location() {
    todo!("get location")
}

async fn handle_post_ping(request: RumaPingRequest) {
    todo!("handle ping request")
}

async fn post_ping(request: RequestExtractor) -> Response {
    let req: RumaPingRequest = RumaPingRequest::try_from_http_request(
        into_bytes_request(request).await,
        &["".to_owned()]
    ).unwrap();

    if MATRIX_HANDLERS_RELEASED {
        // do whatever it takes.
        handle_post_ping(req).await;
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::new(json!({}).to_string())).unwrap();

    response

}

async fn handle_get_thirdparty_protocol(request:RumaGetProtocolRequest) {
    todo!("handling get thirdparty protocol")
}

async fn get_thirdparty_protocol(Path(protocol): Path<String>, request: RequestExtractor) -> Response {
    let req: RumaGetProtocolRequest = RumaGetProtocolRequest::try_from_http_request(
        into_bytes_request(request).await,
        &vec![protocol]
    ).unwrap();

    if MATRIX_HANDLERS_RELEASED {
        // do whatever it takes.
        handle_get_thirdparty_protocol(req).await;
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::new(json!({}).to_string())).unwrap();

    response

}


async fn get_room(Path(room): Path<String>) -> String {
    todo!("room")
}




async fn into_bytes_request(request: Request<Body>) -> axum::http::Request<Bytes>{
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body,usize::MAX).await.unwrap();
    let request = axum::extract::Request::from_parts(
        parts.into(),
        body
    );

    request
}

async fn handle_put_transaction(request: RumaPushEventRequest) {
    todo!("still todo")
}
async fn put_transaction(Path(tid): Path<String>, request: RequestExtractor) -> Response {
    let req: RumaPushEventRequest = RumaPushEventRequest::try_from_http_request(
        into_bytes_request(request).await,
        &vec![tid]
    ).unwrap();

    if MATRIX_HANDLERS_RELEASED {
        // do whatever it takes.
        handle_put_transaction(req).await;
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::new(json!({}).to_string())).unwrap();

    response
}

async fn put_user(Path(user_id): Path<String>) -> String {
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
    use axum::{body::HttpBody, response::Json, extract::FromRequest};
    use ruma::api::MatrixVersion;
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
    async fn matrix_handle_put_user() {
        let hs_token = "test_handle_get_users";
        let request = Request::builder()
            .method("PUT")
            .uri("/_matrix/app/v1/users/1")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::new(json!({}).to_string()))
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

        let r = Request::builder()
            .method("PUT")
            .uri("/_matrix/app/v1/transactions/1")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::new(json!({"events": vec![json!({})], "txn_id": "id".to_owned()}).to_string()))
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::new(json!({}).to_string()))
            .unwrap();
        test_response(hs_token, r, expected).await;
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
            .body(Body::new(json!({}).to_string()))
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
            .body(Body::new(json!({"transaction_id": "mautrix-go_1683636478256400935_123" }).to_string()))
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::new(json!({}).to_string()))
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
            .uri("/_matrix/app/v1/thirdparty/user?userid=@user:example.com")
            .header(header::AUTHORIZATION, format!("Bearer {hs_token}"))
            .body(Body::empty())
            .unwrap();
        let expected = Response::builder()
            .status(StatusCode::OK)
            .body(Body::new(json!({}).to_string()))
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
