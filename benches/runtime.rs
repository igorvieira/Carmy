//! Measures Carmy's own overhead. The fixture tool does no work, so every number
//! here is framework cost (validation, dispatch, serialization, transport).
use axum::body::Body;
use carmy::{Tool, http::ResultDto, runtime::execution_request};
use carmy_benches::{LookupInput, lookup};
use criterion::{Criterion, criterion_group, criterion_main};
use http_body_util::BodyExt;
use serde_json::json;
use std::hint::black_box;
use tower::ServiceExt;

fn tokio() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn arguments() -> serde_json::Value {
    json!({ "sku": "KB-01", "quantity": 2 })
}

fn dispatch(c: &mut Criterion) {
    let rt = tokio();
    let runtime = carmy::Carmy::new().tool(lookup).build().unwrap();
    let mut group = c.benchmark_group("dispatch");
    group.bench_function("direct_tool_call", |b| {
        b.to_async(&rt).iter(|| async {
            let input = LookupInput {
                sku: "KB-01".into(),
                quantity: 2,
            };
            black_box(lookup.execute(Default::default(), input).await.unwrap())
        })
    });
    group.bench_function("runtime_execute", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                runtime
                    .execute(execution_request("lookup", arguments()))
                    .await,
            )
        })
    });
    group.finish();
}

fn serialization(c: &mut Criterion) {
    let rt = tokio();
    let runtime = carmy::Carmy::new().tool(lookup).build().unwrap();
    let result = rt.block_on(runtime.execute(execution_request("lookup", arguments())));
    c.bench_function("json/result_dto", |b| {
        b.iter(|| serde_json::to_vec(&ResultDto::new(black_box(result.clone()), true)).unwrap())
    });
}

fn discovery(c: &mut Criterion) {
    let runtime = carmy::Carmy::new().tool(lookup).build().unwrap();
    let mut group = c.benchmark_group("discovery");
    group.bench_function("schema_generation", |b| {
        b.iter(|| black_box(lookup.metadata()))
    });
    group.bench_function("tool_catalog_json", |b| {
        b.iter(|| serde_json::to_vec(&black_box(&runtime).tools()).unwrap())
    });
    group.finish();
}

fn http(c: &mut Criterion) {
    let rt = tokio();
    let router = carmy::Carmy::new().tool(lookup).router().unwrap();
    let body = json!({ "tool": "lookup", "arguments": arguments() }).to_string();
    c.bench_function("http/execute_roundtrip", |b| {
        b.to_async(&rt).iter(|| async {
            let request = axum::http::Request::post("/agent/execute")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            black_box(response.into_body().collect().await.unwrap().to_bytes())
        })
    });
}

fn concurrency(c: &mut Criterion) {
    let rt = tokio();
    let runtime = carmy::Carmy::new().tool(lookup).build().unwrap();
    c.bench_function("concurrent/64_read_only_executions", |b| {
        b.to_async(&rt).iter(|| async {
            let runs = (0..64).map(|_| {
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    runtime
                        .execute(execution_request("lookup", arguments()))
                        .await
                })
            });
            black_box(futures_util::future::join_all(runs).await)
        })
    });
}

/// What each guarantee costs on its own, with the same libraries the runtime uses.
fn guarantees(c: &mut Criterion) {
    let rt = tokio();
    let metadata = lookup.metadata();
    let input = jsonschema::validator_for(&metadata.input_schema).unwrap();
    let output = jsonschema::validator_for(&metadata.output_schema).unwrap();
    let arguments = arguments();
    let result = json!({ "sku": "KB-01", "available": true, "price_cents": 1299 });
    let mut group = c.benchmark_group("guarantee");
    group.bench_function("input_validation", |b| {
        b.iter(|| input.is_valid(black_box(&arguments)))
    });
    group.bench_function("output_validation", |b| {
        b.iter(|| output.is_valid(black_box(&result)))
    });
    group.bench_function("execution_id", |b| {
        b.iter(|| black_box(format!("exec_{}", uuid::Uuid::new_v4())))
    });
    group.bench_function("cancellation_token", |b| {
        b.iter(|| {
            let token = tokio_util::sync::CancellationToken::new();
            black_box(token.clone().drop_guard()).disarm();
        })
    });
    group.bench_function("deadline_timer", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(tokio::time::timeout(std::time::Duration::from_secs(30), async { 1 }).await)
        })
    });
    group.bench_function("panic_isolation", |b| {
        b.to_async(&rt).iter(|| async {
            use futures_util::FutureExt;
            black_box(
                std::panic::AssertUnwindSafe(async { 1 })
                    .catch_unwind()
                    .await,
            )
        })
    });
    group.bench_function("output_to_json", |b| {
        b.iter(|| {
            black_box(
                serde_json::to_value(carmy_benches::LookupOutput {
                    sku: "KB-01".into(),
                    available: true,
                    price_cents: 1_299,
                })
                .unwrap(),
            )
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    dispatch,
    serialization,
    discovery,
    http,
    concurrency,
    guarantees
);
criterion_main!(benches);
