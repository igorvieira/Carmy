//! The curator pipeline. Every step is a tool, so agents, the console, HTTP clients and
//! the job worker all run the same code with the same guarantees:
//!
//! ```text
//! collect_offers (schedule)  ─▶ process_offer (job per offer, request_id = offer id)
//!   normalize_offer ─▶ calculate_deal ─▶ validate_deal ─▶ generate_affiliate_link
//!   ─▶ publish_premium (now) ─▶ publish_public (after the premium window)
//!   ─▶ publish_twitter (hot deals) ─▶ revalidate_delayed_deal (24 h later)
//! cleanup_expired_deals (schedule)      membership_event (Stripe webhook, enqueued)
//! ```
//!
//! Sources and publishers are fakes, as in a local deal-engine setup, so the example
//! runs anywhere. Swap them for real ones through [`Settings`].
use carmy::{http::Webhook, jobs::Jobs, prelude::*, runtime::execution_request};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

// ------------------------------------------------------------------ domain

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Offer {
    pub id: String,
    pub title: String,
    pub price_cents: u64,
    pub list_price_cents: u64,
    pub in_stock: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Deal {
    pub id: String,
    pub title: String,
    pub price_cents: u64,
    pub discount_percent: u8,
    /// 0–100; deals at 80 or more also go to Twitter.
    pub score: u8,
    pub affiliate_url: String,
    pub published_to: Vec<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// What the app needs from the outside. Fakes by default.
#[derive(Clone)]
pub struct Settings {
    pub offers: Vec<Offer>,
    /// How long premium members see a deal before the public does.
    pub premium_window: Duration,
    pub stripe_secret: String,
    pub retry: carmy::jobs::RetryPolicy,
}

impl Settings {
    /// Fake offers, a 30-second premium window, and `STRIPE_WEBHOOK_SECRET`.
    pub fn from_env() -> Self {
        Self {
            offers: sample_offers(),
            premium_window: Duration::from_secs(30),
            stripe_secret: std::env::var("STRIPE_WEBHOOK_SECRET")
                .unwrap_or_else(|_| "whsec_dev".into()),
            retry: carmy::jobs::RetryPolicy::default(),
        }
    }
}

pub fn sample_offers() -> Vec<Offer> {
    let offer = |id: &str, title: &str, price: u64, list: u64, in_stock: bool| Offer {
        id: id.into(),
        title: title.into(),
        price_cents: price,
        list_price_cents: list,
        in_stock,
        url: format!("https://shop.example/{id}"),
    };
    vec![
        offer("kb-01", "  Mechanical Keyboard  ", 4_900, 12_900, true),
        offer("ms-02", "wireless mouse", 4_400, 4_900, true),
        offer("mn-03", "4K Monitor", 19_900, 39_900, false),
    ]
}

/// Where offers come from. The fake returns the configured list.
#[derive(Clone)]
pub struct Source(pub Arc<Vec<Offer>>);

/// Where deals go. The fake records every publication, and the first premium
/// publication is refused once, to show a retry in action.
#[derive(Default)]
pub struct Publishers {
    pub premium: Mutex<Vec<String>>,
    pub public: Mutex<Vec<String>>,
    pub twitter: Mutex<Vec<String>>,
    pub refusals_left: Mutex<u32>,
}

impl Publishers {
    pub fn refusing(times: u32) -> Self {
        Self {
            refusals_left: Mutex::new(times),
            ..Self::default()
        }
    }
}

/// Deals and memberships, in memory. A real app keeps them in its database.
#[derive(Default)]
pub struct Store {
    pub deals: Mutex<BTreeMap<String, Deal>>,
    pub members: Mutex<BTreeMap<String, String>>,
}

// ------------------------------------------------------------------ pipeline steps
//
// Plain functions hold the logic; the tools below expose them. `process_offer` chains
// the functions in-process, so one job runs the whole pipeline for one offer.

pub fn normalize(mut offer: Offer) -> Offer {
    let title = offer.title.trim();
    let words = title.split_whitespace().map(|w| {
        let mut chars = w.chars();
        match chars.next() {
            Some(first) => {
                first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
            }
            None => String::new(),
        }
    });
    offer.title = words.collect::<Vec<_>>().join(" ");
    offer.list_price_cents = offer.list_price_cents.max(offer.price_cents);
    offer
}

pub fn score(offer: &Offer) -> Scored {
    let discount = 100 - (offer.price_cents * 100 / offer.list_price_cents.max(1)).min(100);
    // Discount carries most of the weight; cheap items get a small bonus.
    let bonus = if offer.price_cents < 5_000 { 10 } else { 0 };
    Scored {
        discount_percent: discount as u8,
        score: (discount * 3 / 2 + bonus).min(100) as u8,
    }
}

pub fn validate(offer: &Offer, scored: &Scored) -> Validated {
    let reason = if !offer.in_stock {
        Some("out of stock")
    } else if scored.discount_percent < 10 {
        Some("discount under 10%")
    } else {
        None
    };
    Validated {
        ok: reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}

pub fn affiliate_link(url: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}tag=curator-21")
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct Scored {
    pub discount_percent: u8,
    pub score: u8,
}

// ------------------------------------------------------------------ tools

#[derive(Serialize, JsonSchema)]
pub struct Collected {
    pub offers: usize,
    pub enqueued: usize,
}

#[carmy::tool(
    description = "Fetch offers from every source and queue one process_offer per offer",
    effect = "read",
    register = false
)]
async fn collect_offers(
    State(source): State<Source>,
    State(jobs): State<Jobs>,
) -> AgentResult<Collected> {
    let mut enqueued = 0;
    for offer in source.0.iter() {
        // The offer id is the job's identity: collecting twice never processes twice.
        let request = execution_request("process_offer", json!({ "offer": offer }))
            .with_request_id(format!("process-{}", offer.id));
        jobs.enqueue(request).await?;
        enqueued += 1;
    }
    Ok(Collected {
        offers: source.0.len(),
        enqueued,
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct OfferInput {
    pub offer: Offer,
}

#[carmy::tool(
    description = "Trim titles, fix casing and clamp prices",
    effect = "none",
    idempotent = true,
    parallel_safe = true,
    register = false
)]
async fn normalize_offer(input: OfferInput) -> AgentResult<Offer> {
    Ok(normalize(input.offer))
}

#[carmy::tool(
    description = "Score an offer deterministically from its discount",
    effect = "none",
    idempotent = true,
    parallel_safe = true,
    register = false
)]
async fn calculate_deal(input: OfferInput) -> AgentResult<Scored> {
    Ok(score(&input.offer))
}

#[derive(Deserialize, JsonSchema)]
pub struct ValidateInput {
    pub offer: Offer,
    pub scored: Scored,
}

#[derive(Serialize, JsonSchema)]
pub struct Validated {
    pub ok: bool,
    pub reason: Option<String>,
}

#[carmy::tool(
    description = "Reject offers that are out of stock or not real discounts",
    effect = "read",
    idempotent = true,
    parallel_safe = true,
    register = false
)]
async fn validate_deal(input: ValidateInput) -> AgentResult<Validated> {
    Ok(validate(&input.offer, &input.scored))
}

#[derive(Deserialize, JsonSchema)]
pub struct LinkInput {
    pub url: String,
}

#[carmy::tool(
    description = "Tag a product URL with the affiliate id",
    effect = "none",
    idempotent = true,
    parallel_safe = true,
    register = false
)]
async fn generate_affiliate_link(input: LinkInput) -> AgentResult<String> {
    Ok(affiliate_link(&input.url))
}

#[derive(Serialize, JsonSchema)]
pub struct Processed {
    pub deal_id: String,
    pub published: bool,
    pub reason: Option<String>,
}

#[carmy::tool(
    description = "Run one offer through the pipeline and schedule its publications",
    effect = "write",
    parallel_safe = true,
    register = false
)]
async fn process_offer(
    State(store): State<Arc<Store>>,
    State(settings): State<Settings>,
    State(jobs): State<Jobs>,
    input: OfferInput,
) -> AgentResult<Processed> {
    let offer = normalize(input.offer);
    let scored = score(&offer);
    let validated = validate(&offer, &scored);
    if !validated.ok {
        return Ok(Processed {
            deal_id: offer.id,
            published: false,
            reason: validated.reason,
        });
    }
    let affiliate_url = affiliate_link(&offer.url);
    let deal = Deal {
        id: offer.id.clone(),
        title: offer.title,
        price_cents: offer.price_cents,
        discount_percent: scored.discount_percent,
        score: scored.score,
        affiliate_url,
        published_to: Vec::new(),
        expires_at: chrono::Utc::now() + chrono::Duration::days(7),
    };
    store
        .deals
        .lock()
        .unwrap()
        .insert(deal.id.clone(), deal.clone());

    // Each publication is its own job with its own identity: retries and duplicate
    // collections can never publish twice.
    let publish = |channel: &str| {
        execution_request(format!("publish_{channel}"), json!({ "deal_id": deal.id }))
            .with_request_id(format!("publish-{}-{channel}", deal.id))
    };
    jobs.enqueue(publish("premium")).await?;
    jobs.enqueue_after(publish("public"), settings.premium_window)
        .await?;
    if deal.score >= 80 {
        jobs.enqueue_after(publish("twitter"), settings.premium_window)
            .await?;
    }
    jobs.enqueue_after(
        execution_request("revalidate_delayed_deal", json!({ "deal_id": deal.id }))
            .with_request_id(format!("revalidate-{}", deal.id)),
        Duration::from_secs(24 * 3600),
    )
    .await?;
    Ok(Processed {
        deal_id: deal.id,
        published: true,
        reason: None,
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct DealRef {
    pub deal_id: String,
}

fn publish_to(
    store: &Store,
    channel: &str,
    log: &Mutex<Vec<String>>,
    id: &str,
) -> AgentResult<Deal> {
    let mut deals = store.deals.lock().unwrap();
    let deal = deals.get_mut(id).ok_or_else(|| {
        AgentError::new(
            "DEAL_NOT_FOUND",
            format!("no deal `{id}`"),
            ErrorCategory::NotFound,
        )
    })?;
    log.lock().unwrap().push(id.to_owned());
    deal.published_to.push(channel.to_owned());
    Ok(deal.clone())
}

#[carmy::tool(
    description = "Publish a deal to premium members",
    effect = "external_write",
    parallel_safe = true,
    register = false
)]
async fn publish_premium(
    State(store): State<Arc<Store>>,
    State(publishers): State<Arc<Publishers>>,
    input: DealRef,
) -> AgentResult<Deal> {
    {
        let mut left = publishers.refusals_left.lock().unwrap();
        if *left > 0 {
            *left -= 1;
            return Err(AgentError::new(
                "PUBLISHER_BUSY",
                "the premium channel is rate limited",
                ErrorCategory::Capacity,
            )
            .retryable(Some(0)));
        }
    }
    publish_to(&store, "premium", &publishers.premium, &input.deal_id)
}

#[carmy::tool(
    description = "Publish a deal to the public site",
    effect = "external_write",
    parallel_safe = true,
    register = false
)]
async fn publish_public(
    State(store): State<Arc<Store>>,
    State(publishers): State<Arc<Publishers>>,
    input: DealRef,
) -> AgentResult<Deal> {
    publish_to(&store, "public", &publishers.public, &input.deal_id)
}

#[carmy::tool(
    description = "Post a hot deal on Twitter",
    effect = "external_write",
    parallel_safe = true,
    register = false
)]
async fn publish_twitter(
    State(store): State<Arc<Store>>,
    State(publishers): State<Arc<Publishers>>,
    input: DealRef,
) -> AgentResult<Deal> {
    publish_to(&store, "twitter", &publishers.twitter, &input.deal_id)
}

#[carmy::tool(
    description = "A day later, drop a deal whose offer is gone",
    effect = "write",
    parallel_safe = true,
    register = false
)]
async fn revalidate_delayed_deal(
    State(store): State<Arc<Store>>,
    State(source): State<Source>,
    input: DealRef,
) -> AgentResult<bool> {
    let still_offered = source.0.iter().any(|o| o.id == input.deal_id && o.in_stock);
    if !still_offered {
        store.deals.lock().unwrap().remove(&input.deal_id);
    }
    Ok(still_offered)
}

#[carmy::tool(
    description = "Remove deals past their expiry",
    effect = "write",
    register = false
)]
async fn cleanup_expired_deals(State(store): State<Arc<Store>>) -> AgentResult<usize> {
    let now = chrono::Utc::now();
    let mut deals = store.deals.lock().unwrap();
    let before = deals.len();
    deals.retain(|_, d| d.expires_at > now);
    Ok(before - deals.len())
}

#[derive(Deserialize, JsonSchema)]
pub struct StripeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub data: serde_json::Value,
}

#[carmy::tool(
    description = "Apply a Stripe subscription event to a membership",
    effect = "write",
    parallel_safe = true,
    register = false
)]
async fn membership_event(
    State(store): State<Arc<Store>>,
    input: StripeEvent,
) -> AgentResult<String> {
    let customer = input.data["object"]["customer"]
        .as_str()
        .ok_or_else(|| {
            AgentError::new(
                "INVALID_EVENT",
                "the event has no customer",
                ErrorCategory::Validation,
            )
        })?
        .to_owned();
    let status = match input.kind.as_str() {
        "customer.subscription.created" | "customer.subscription.updated" => "premium",
        "customer.subscription.deleted" => "free",
        _ => "ignored",
    };
    if status != "ignored" {
        store
            .members
            .lock()
            .unwrap()
            .insert(customer.clone(), status.into());
    }
    Ok(status.into())
}

// ------------------------------------------------------------------ app

/// The application: tools, state, schedules, webhook, readiness and its own routes.
pub fn app(settings: Settings) -> Carmy {
    app_with(
        settings,
        Arc::new(Store::default()),
        Arc::new(Publishers::refusing(1)),
    )
}

/// [`app`] with explicit store and publishers, so tests can look inside.
pub fn app_with(settings: Settings, store: Arc<Store>, publishers: Arc<Publishers>) -> Carmy {
    let site = axum::Router::new().route(
        "/deals",
        axum::routing::get({
            let store = store.clone();
            move || {
                let deals: Vec<Deal> = store.deals.lock().unwrap().values().cloned().collect();
                async move { axum::Json(deals) }
            }
        }),
    );
    let app = Carmy::new()
        .name("curator")
        .state(Source(Arc::new(settings.offers.clone())))
        .state(store)
        .state(publishers)
        .state(settings.clone())
        .retry(settings.retry)
        .tool(collect_offers)
        .tool(normalize_offer)
        .tool(calculate_deal)
        .tool(validate_deal)
        .tool(generate_affiliate_link)
        .tool(process_offer)
        .tool(publish_premium)
        .tool(publish_public)
        .tool(publish_twitter)
        .tool(revalidate_delayed_deal)
        .tool(cleanup_expired_deals)
        .tool(membership_event)
        .schedule("collect", "0 */5 * * * * *", || {
            execution_request("collect_offers", json!({}))
        })
        .schedule("cleanup", "0 0 3 * * * *", || {
            execution_request("cleanup_expired_deals", json!({}))
        })
        .webhook(
            "/webhooks/stripe",
            Webhook::stripe(settings.stripe_secret.clone())
                .tool("membership_event")
                .enqueue(),
        )
        .routes(site)
        .require_worker(Duration::from_secs(60));
    with_database(app)
}

#[cfg(feature = "postgres")]
fn with_database(app: Carmy) -> Carmy {
    use carmy::postgres::{
        PostgresAudit, PostgresIdempotencyStore, PostgresJobStore, connect, migrate,
    };
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return app;
    };
    let pool = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let pool = connect(&url).await.expect("DATABASE_URL is reachable");
            migrate(&pool).await.expect("migrations apply");
            pool
        })
    });
    app.jobs(Arc::new(PostgresJobStore::new(pool.clone())))
        .idempotency_store(Arc::new(PostgresIdempotencyStore::new(pool.clone())))
        .sink(Arc::new(PostgresAudit::new(pool.clone())))
        .ready("database", move || carmy::postgres::ready(pool.clone()))
        .command("migrate", |_| Box::pin(async { Ok(()) }))
}

#[cfg(not(feature = "postgres"))]
fn with_database(app: Carmy) -> Carmy {
    app
}
