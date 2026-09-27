//! An OpenAPI description's operations served as the app's own tools.
#![cfg(feature = "openapi")]
use axum::{Json, Router, body::Body, http::Request, routing::get};
use carmy::prelude::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn an_app_serves_an_apis_operations_as_tools() {
    let api = Router::new().route("/weather", get(|| async { Json(json!({"temp_c": 21})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });

    let spec = r#"{"openapi": "3.1.0", "info": {"title": "w", "version": "1"},
      "paths": {"/weather": {"get": {"operationId": "currentWeather", "summary": "Weather now",
        "responses": {"200": {"description": "ok"}}}}}}"#;
    let tools = carmy::openapi::OpenApi::from_json(spec)
        .unwrap()
        .base_url(base)
        .prefix("weather_")
        .tools()
        .unwrap();
    let app = Carmy::new().tools(tools).router().unwrap();
    let response = app
        .oneshot(
            Request::post("/agent/execute")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"tool": "weather_currentWeather", "arguments": {}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["temp_c"], 21);
    assert_eq!(
        body["_agent"]["cacheable"], true,
        "a GET is a read, idempotent"
    );
}
