use core::fmt;
use std::marker::PhantomData;

use axum::{
    body::Body,
    http::{header, HeaderValue, Request, Response, StatusCode},
    routing::get,
    Router,
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
            .route("/_matrix/", get(|| async {})) // TODO: request method
            .route_layer(ValidateRequestHeaderLayer::custom(MatrixBearer::new(
                hs_token,
            )));
    }
    // pub async fn serve(&self, listener: TcpListener) {
    //     axum::serve(listener, self.app).await.unwrap();
    // }
}

#[cfg(test)]
mod tests {
    use axum::response::Json;
    use serde_json::{json, Value};
    use tower_service::Service;

    use super::*;

    // TODO(nemo): Clean up test
    #[tokio::test]
    async fn matrix_handle_invalid_token() {
        let hs_token = "abcd";
        let request = Request::builder()
            .method("GET")
            .uri("/_matrix/")
            .header("Authentication", "Bearer dcba")
            .body(Body::empty())
            .unwrap();
        let mut http = Http::new();
        http.add_matrix_route(hs_token);
        // TODO(nemo): Decide if using tower_service::oneshot() is better
        // than call().
        let res = http.app.call(request).await.unwrap();
        let expected = Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Json(
                json!({ "errcode": "M_FORBIDDEN", "error": "Bad token supplied" }),
            ))
            .unwrap();
        assert_eq!(res.status(), expected.status());
        let res_body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let res_json: Value = serde_json::from_slice(&res_body).unwrap();
        assert_eq!(res_json, expected.body().0);
    }
}
