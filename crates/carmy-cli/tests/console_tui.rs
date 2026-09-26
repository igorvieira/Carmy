use carmy_cli::console::{
    app::{App, Focus, Modal, Phase},
    ui,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::{Value, json};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn catalog() -> Value {
    json!({"tools": [
        {"name": "create_order", "description": "Place an order", "effect": "write",
         "idempotent": false, "parallel_safe": true, "confirmation": "none",
         "input_schema": {"type": "object", "properties": {"sku": {"type": "string"}}, "required": ["sku"]},
         "output_schema": {"type": "object", "properties": {"order_id": {"type": "integer"}}}},
        {"name": "cancel_order", "description": "Cancel an order", "effect": "destructive",
         "idempotent": true, "parallel_safe": true, "confirmation": "required",
         "input_schema": {"type": "object", "properties": {"order_id": {"type": "integer"}}},
         "output_schema": {"type": "object", "properties": {"cancelled": {"type": "boolean"}}}}
    ]})
}
/// A connected app with the catalog loaded.
fn ready() -> App {
    let mut app = App::new();
    let requests =
        app.apply(json!({"event":"ready","protocol":"carmy-console/1","server":"shop","tools":2}));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["op"], "tools");
    let id = requests[0]["id"].clone();
    app.apply(json!({"id": id, "ok": true, "result": catalog()}));
    app
}
fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn loads_the_catalog_with_prefilled_arguments() {
    let app = ready();
    assert_eq!(app.phase, Phase::Ready);
    assert_eq!(app.tools.len(), 2);
    assert_eq!(app.arguments().text, r#"{"sku":""}"#);
    assert!(app.locked(&app.tools[1]));
    let screen = screen(&app);
    for text in [
        "shop tools",
        "create_order",
        "cancel_order",
        "🔒",
        "sku: string",
        "effect: write",
    ] {
        assert!(screen.contains(text), "missing {text:?} in:\n{screen}");
    }
}

#[test]
fn runs_a_call_and_shows_replays() {
    let mut app = ready();
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Arguments);
    app.handle_key(ctrl('u'));
    for c in r#"{"sku":"KB-01"}"#.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Tab));
    for c in "order-1".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let requests = app.handle_key(key(KeyCode::Enter));
    assert_eq!(requests.len(), 1);
    let call = &requests[0];
    assert_eq!(call["op"], "call");
    assert_eq!(call["arguments"], json!({"sku": "KB-01"}));
    assert_eq!(call["request_id"], "order-1");
    app.apply(json!({"id": call["id"], "ok": true, "result": {
        "execution_id": "exec_1", "status": "completed", "data": {"order_id": 7},
        "replayed": true, "duration_ms": 0.4
    }}));
    let result = app.result.as_ref().unwrap();
    assert!(result.replayed);
    assert_eq!(app.focus, Focus::Result);
    let screen = screen(&app);
    assert!(
        screen.contains("replayed") && screen.contains("\"order_id\": 7"),
        "{screen}"
    );
    // `r` reruns the last call with the same request_id.
    let again = app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(again[0]["request_id"], "order-1");
}

#[test]
fn errors_show_code_and_suggested_action() {
    let mut app = ready();
    app.handle_key(key(KeyCode::Tab));
    let requests = app.handle_key(key(KeyCode::Enter));
    app.apply(json!({"id": requests[0]["id"], "ok": true, "result": {
        "status": "failed", "duration_ms": 0.1, "replayed": false,
        "error": {"code": "PRODUCT_NOT_FOUND", "message": "No product has this SKU",
                  "category": "not_found", "recoverable": true, "retryable": false,
                  "suggested_action": "search_products"}
    }}));
    let screen = screen(&app);
    assert!(
        screen.contains("PRODUCT_NOT_FOUND") && screen.contains("search_products"),
        "{screen}"
    );
}

#[test]
fn invalid_json_is_caught_before_sending() {
    let mut app = ready();
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Char('x')));
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert!(app.status.contains("not valid JSON"));
}

#[test]
fn confirmation_is_granted_and_revoked_through_a_modal() {
    let mut app = ready();
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.tool().unwrap().name, "cancel_order");
    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(
        app.modal,
        Some(Modal::Confirm {
            tool: "cancel_order".into(),
            grant: true
        })
    );
    assert!(screen(&app).contains("Grant confirm:cancel_order"));
    let requests = app.handle_key(key(KeyCode::Char('y')));
    assert_eq!(requests[0]["op"], "confirm");
    app.apply(json!({"id": requests[0]["id"], "ok": true, "result": {"tool": "cancel_order", "confirmed": true}}));
    assert!(!app.locked(app.tool().unwrap()));
    assert!(!screen(&app).contains("🔒"));
    // Pressing c again offers to revoke; `n` keeps the grant.
    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(
        app.modal,
        Some(Modal::Confirm {
            tool: "cancel_order".into(),
            grant: false
        })
    );
    assert!(app.handle_key(key(KeyCode::Char('n'))).is_empty());
    assert!(!app.locked(app.tool().unwrap()));
}

#[test]
fn history_reruns_past_calls() {
    let mut app = ready();
    app.handle_key(key(KeyCode::Tab));
    let first = app.handle_key(key(KeyCode::Enter));
    app.apply(json!({"id": first[0]["id"], "ok": true, "result": {"status": "completed", "data": {}, "replayed": false, "duration_ms": 1.0}}));
    app.handle_key(key(KeyCode::Char('h')));
    assert!(matches!(app.modal, Some(Modal::History { selected: 0 })));
    assert!(screen(&app).contains("History"));
    let again = app.handle_key(key(KeyCode::Enter));
    assert_eq!(again[0]["tool"], "create_order");
    assert!(app.modal.is_none());
}

#[test]
fn startup_failures_are_reported() {
    let mut app = App::new();
    app.log("error[E0425]: cannot find value `x`".into());
    app.exited(Some(101));
    assert!(matches!(app.phase, Phase::Failed(_)));
    assert!(screen(&app).contains("E0425"));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.quit);
}

#[test]
fn request_ids_belong_to_one_tool() {
    let mut app = ready();
    app.handle_key(key(KeyCode::BackTab)); // tools -> result
    app.handle_key(key(KeyCode::BackTab)); // result -> request_id
    for c in "order-1".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(app.request_id().text, "order-1");
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Down));
    // Reusing order-1 with another tool would be an idempotency conflict.
    assert_eq!(app.request_id().text, "");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.request_id().text, "order-1");
}
