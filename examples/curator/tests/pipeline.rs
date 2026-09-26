//! End to end: offers enter through the fake source and leave published by the fake
//! publishers, once each, even with two workers, a refused publication and a
//! redelivered webhook.
use axum::{body::Body, http::Request};
use carmy::jobs::RetryPolicy;
use curator::{Publishers, Settings, Store, app_with, sample_offers};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn settings() -> Settings {
    Settings {
        offers: sample_offers(),
        premium_window: Duration::from_millis(0),
        stripe_secret: "whsec_test".into(),
        retry: RetryPolicy {
            max_attempts: 3,
            base: Duration::from_millis(1),
            cap: Duration::from_millis(1),
        },
    }
}

fn stripe_delivery(secret: &str, body: &str) -> Request<Body> {
    let t = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    let signature = format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()));
    Request::post("/webhooks/stripe")
        .header("stripe-signature", signature)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// Two workers drain the queue together until nothing is due for a while.
async fn drain(jobs: &carmy::jobs::Jobs) {
    let mut idle_rounds = 0;
    while idle_rounds < 5 {
        let (a, b) = tokio::join!(jobs.run_due(8), jobs.run_due(8));
        if a.unwrap() + b.unwrap() == 0 {
            idle_rounds += 1;
            tokio::time::sleep(Duration::from_millis(5)).await;
        } else {
            idle_rounds = 0;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn offers_become_deals_and_are_published_exactly_once() {
    let store = Arc::new(Store::default());
    let publishers = Arc::new(Publishers::refusing(1));
    let (router, jobs) = app_with(settings(), store.clone(), publishers.clone())
        .router_and_jobs()
        .unwrap();

    // Two collections (a schedule tick and a manual run) queue each offer once.
    for _ in 0..2 {
        jobs.enqueue(carmy::runtime::execution_request(
            "collect_offers",
            json!({}),
        ))
        .await
        .unwrap();
    }
    drain(&jobs).await;

    let deals = store.deals.lock().unwrap().clone();
    assert_eq!(
        deals.len(),
        2,
        "the out-of-stock monitor never became a deal"
    );
    let keyboard = &deals["kb-01"];
    assert_eq!(keyboard.title, "Mechanical Keyboard");
    assert_eq!(keyboard.discount_percent, 63);
    assert!(keyboard.score >= 80, "score {}", keyboard.score);
    assert_eq!(
        keyboard.affiliate_url,
        "https://shop.example/kb-01?tag=curator-21"
    );
    let mouse = &deals["ms-02"];
    assert_eq!(mouse.discount_percent, 11);
    assert!(mouse.score < 80);

    let premium = publishers.premium.lock().unwrap().clone();
    let public = publishers.public.lock().unwrap().clone();
    let twitter = publishers.twitter.lock().unwrap().clone();
    assert_eq!(
        sorted(premium),
        vec!["kb-01", "ms-02"],
        "premium: once per deal, after one refusal"
    );
    assert_eq!(sorted(public), vec!["kb-01", "ms-02"]);
    assert_eq!(twitter, vec!["kb-01"], "only hot deals reach Twitter");
    assert_eq!(*publishers.refusals_left.lock().unwrap(), 0);
    assert!(jobs.dead_letters(10).await.unwrap().is_empty());

    // The app's own route sees the deals; the agent routes and readiness are there too.
    let response = router
        .clone()
        .oneshot(Request::get("/deals").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let listed: Vec<Value> = json_body(response).await.as_array().unwrap().clone();
    assert_eq!(listed.len(), 2);
    let ready = router
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status(), 200, "run_due counts as a worker tick");

    // A Stripe event, delivered twice, changes the membership once.
    let event = json!({
        "id": "evt_sub_1",
        "type": "customer.subscription.created",
        "data": { "object": { "customer": "cus_ada" } }
    })
    .to_string();
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(stripe_delivery("whsec_test", &event))
            .await
            .unwrap();
        assert_eq!(response.status(), 202);
    }
    drain(&jobs).await;
    let members = store.members.lock().unwrap().clone();
    assert_eq!(members.get("cus_ada").map(String::as_str), Some("premium"));
    let audit_runs = jobs.get(&carmy::jobs::JobId("nope".into())).await.unwrap();
    assert!(audit_runs.is_none());

    let forged = router
        .oneshot(stripe_delivery("wrong", &event))
        .await
        .unwrap();
    assert_eq!(forged.status(), 401);
}

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
