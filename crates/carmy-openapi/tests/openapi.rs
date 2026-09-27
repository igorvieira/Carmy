//! A small pet API, described in OpenAPI and served locally, called through Carmy.
use axum::{
    Json, Router,
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use carmy_core::*;
use carmy_openapi::OpenApi;
use carmy_runtime::{Runtime, execution_request};
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: &str = r##"{
  "openapi": "3.0.3",
  "info": {"title": "Pets", "version": "1"},
  "servers": [{"url": "http://placeholder"}],
  "paths": {
    "/pets": {
      "get": {
        "operationId": "listPets",
        "summary": "List pets",
        "parameters": [
          {"name": "limit", "in": "query", "schema": {"type": "integer", "maximum": 100}},
          {"name": "X-Trace", "in": "header", "schema": {"type": "string"}}
        ],
        "responses": {"200": {"description": "ok", "content": {"application/json": {
          "schema": {"type": "array", "items": {"$ref": "#/components/schemas/Pet"}}}}}}
      },
      "post": {
        "operationId": "createPet",
        "summary": "Create a pet",
        "requestBody": {"$ref": "#/components/requestBodies/NewPet"},
        "responses": {"201": {"description": "created"}}
      }
    },
    "/pets/{petId}": {
      "parameters": [{"$ref": "#/components/parameters/PetId"}],
      "get": {"operationId": "getPet", "responses": {"200": {"description": "ok"}}},
      "delete": {"operationId": "deletePet", "responses": {"204": {"description": "gone"}}}
    },
    "/search": {
      "post": {
        "operationId": "searchPets",
        "x-carmy-effect": "read",
        "requestBody": {"content": {"application/json": {"schema": {"type": "object"}}}},
        "responses": {"200": {"description": "ok"}}
      }
    },
    "/busy": {"get": {"operationId": "busy", "responses": {"200": {"description": "ok"}}}}
  },
  "components": {
    "parameters": {
      "PetId": {"name": "petId", "in": "path", "required": true, "schema": {"type": "string"}}
    },
    "requestBodies": {
      "NewPet": {"required": true, "content": {"application/json": {
        "schema": {"$ref": "#/components/schemas/NewPet"}}}}
    },
    "schemas": {
      "NewPet": {"type": "object", "required": ["name"], "properties": {
        "name": {"type": "string"},
        "tag": {"type": "string", "nullable": true},
        "owner": {"$ref": "#/components/schemas/Owner"}}},
      "Owner": {"type": "object", "properties": {"email": {"type": "string"}}},
      "Pet": {"type": "object", "properties": {"id": {"type": "string"}, "name": {"type": "string"}}}
    }
  }
}"##;

async fn serve() -> String {
    let app = Router::new()
        .route(
            "/pets",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                Json(json!({"pets": [{"id": "1", "name": "Rex"}], "limit": q.get("limit")}))
            })
            .post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                let key = headers
                    .get("idempotency-key")
                    .map(|v| v.to_str().unwrap().to_owned());
                let auth = headers
                    .get("authorization")
                    .map(|v| v.to_str().unwrap().to_owned());
                (
                    StatusCode::CREATED,
                    Json(json!({"created": body, "key": key, "auth": auth})),
                )
            }),
        )
        .route(
            "/pets/{id}",
            get(|Path(id): Path<String>| async move {
                if id == "missing" {
                    return (StatusCode::NOT_FOUND, Json(json!({"message": "no pet"})))
                        .into_response();
                }
                Json(json!({"id": id})).into_response()
            })
            .delete(|| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/search",
            axum::routing::post(|Json(b): Json<Value>| async move { Json(b) }),
        )
        .route(
            "/busy",
            get(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "7")],
                    "slow down",
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

async fn runtime() -> Runtime {
    let tools = OpenApi::from_json(SPEC)
        .unwrap()
        .base_url(serve().await)
        .header("Authorization", "Bearer secret")
        .tools()
        .unwrap();
    let mut runtime = Runtime::new();
    for tool in tools {
        runtime.register(tool).unwrap();
    }
    runtime
}

#[tokio::test]
async fn operations_become_tools_with_effects_and_schemas() {
    let rt = runtime().await;
    let names: Vec<String> = rt.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(
        names,
        [
            "busy",
            "createPet",
            "deletePet",
            "getPet",
            "listPets",
            "searchPets"
        ]
    );

    let list = rt.metadata("listPets").unwrap();
    assert_eq!(list.effect, Effect::Read);
    assert!(list.idempotent);
    assert!(
        list.input_schema["properties"].get("X-Trace").is_none(),
        "headers are the host's"
    );
    let create = rt.metadata("createPet").unwrap();
    assert_eq!(create.effect, Effect::ExternalWrite);
    assert!(!create.idempotent);
    assert_eq!(create.input_schema["required"], json!(["body"]));
    assert_eq!(
        create.input_schema["$defs"]["NewPet"]["properties"]["tag"]["type"],
        json!(["string", "null"])
    );
    assert!(
        create.input_schema["$defs"].get("Owner").is_some(),
        "references are followed"
    );
    assert!(
        create.input_schema["$defs"].get("Pet").is_none(),
        "only what the tool uses"
    );
    let delete = rt.metadata("deletePet").unwrap();
    assert_eq!(delete.effect, Effect::Destructive);
    assert_eq!(delete.confirmation, Confirmation::Required);
    assert_eq!(
        rt.metadata("searchPets").unwrap().effect,
        Effect::Read,
        "x-carmy-effect"
    );
}

#[tokio::test]
async fn calls_reach_the_api_with_credentials_and_replay_safe_writes() {
    let rt = runtime().await;
    let list = rt
        .execute(execution_request("listPets", json!({"limit": 5})))
        .await;
    assert_eq!(list.outcome.unwrap()["limit"], "5");

    let request = execution_request(
        "createPet",
        json!({"body": {"name": "Rex", "tag": null, "owner": {"email": "a@b.c"}}}),
    )
    .with_request_id("create-rex");
    let created = rt.execute(request).await.outcome.unwrap();
    assert_eq!(created["created"]["name"], "Rex");
    assert_eq!(
        created["key"], "create-rex",
        "request_id travels as Idempotency-Key"
    );
    assert_eq!(created["auth"], "Bearer secret");

    // The schema guards the API: no body, or a wrong type, never leaves the process.
    let missing = rt.execute(execution_request("createPet", json!({}))).await;
    assert_eq!(missing.outcome.unwrap_err().code, "INVALID_ARGUMENTS");
    let wrong = rt
        .execute(execution_request("listPets", json!({"limit": 500})))
        .await;
    assert_eq!(wrong.outcome.unwrap_err().code, "INVALID_ARGUMENTS");

    // Path values stay in their segment.
    let odd = rt
        .execute(execution_request("getPet", json!({"petId": "a/../b"})))
        .await;
    assert_eq!(odd.outcome.unwrap()["id"], "a/../b");
}

#[tokio::test]
async fn api_errors_become_structured_errors_agents_can_act_on() {
    let rt = runtime().await;
    let missing = rt
        .execute(execution_request("getPet", json!({"petId": "missing"})))
        .await
        .outcome
        .unwrap_err();
    assert_eq!(missing.code, "UPSTREAM_NOT_FOUND");
    assert_eq!(missing.category, ErrorCategory::NotFound);
    assert_eq!(
        missing.details.as_ref().unwrap()["body"]["message"],
        "no pet"
    );

    let busy = rt
        .execute(execution_request("busy", json!({})))
        .await
        .outcome
        .unwrap_err();
    assert_eq!(busy.code, "UPSTREAM_RATE_LIMITED");
    assert!(busy.retryable);
    assert_eq!(busy.retry_after, Some(7));

    // Destructive operations need confirmation, like any Carmy tool.
    let unconfirmed = rt
        .execute(execution_request("deletePet", json!({"petId": "1"})))
        .await
        .outcome
        .unwrap_err();
    assert_eq!(unconfirmed.code, "CONFIRMATION_REQUIRED");
    let mut confirmed = execution_request("deletePet", json!({"petId": "1"}));
    confirmed
        .context
        .permissions
        .insert("confirm:deletePet".into());
    assert_eq!(rt.execute(confirmed).await.outcome.unwrap(), Value::Null);

    let unreachable = OpenApi::from_json(SPEC)
        .unwrap()
        .base_url("http://127.0.0.1:1")
        .only(["busy"])
        .tools()
        .unwrap();
    let mut down = Runtime::new();
    for tool in unreachable {
        down.register(tool).unwrap();
    }
    let error = down
        .execute(execution_request("busy", json!({})))
        .await
        .outcome
        .unwrap_err();
    assert_eq!(error.code, "UPSTREAM_UNREACHABLE");
    assert!(error.retryable);
}

#[test]
fn only_openapi_3_is_accepted() {
    assert!(OpenApi::from_json(r#"{"swagger": "2.0"}"#).is_err());
    assert!(OpenApi::from_json("not json").is_err());
}
