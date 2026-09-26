#![no_main]
//! Arbitrary request bodies into POST /agent/execute: never a panic, always a JSON
//! response with a status the adapter defines.
mod common;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use libfuzzer_sys::fuzz_target;
use tower::ServiceExt;

fuzz_target!(|data: &[u8]| {
    common::block_on(async {
        let router = carmy::http::router(common::runtime(), "fuzz");
        let request = Request::post("/agent/execute")
            .header("content-type", "application/json")
            .body(Body::from(data.to_vec()))
            .expect("a request");
        let response = router.oneshot(request).await.expect("the router is infallible");
        let status = response.status().as_u16();
        assert!(
            matches!(status, 200 | 400 | 403 | 404 | 408 | 409 | 413 | 500 | 503 | 504),
            "unexpected status {status}"
        );
        let body = response.into_body().collect().await.expect("a body").to_bytes();
        serde_json::from_slice::<serde_json::Value>(&body).expect("the body is JSON");
    });
});
