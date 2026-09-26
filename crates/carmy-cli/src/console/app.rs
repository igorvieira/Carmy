//! The console's state and how keys and protocol responses change it. No terminal here.
use super::schema;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub effect: String,
    pub idempotent: bool,
    pub parallel_safe: bool,
    pub confirmation: bool,
    pub input: String,
    pub output: String,
    pub skeleton: String,
}

impl ToolInfo {
    fn from_metadata(m: &Value) -> Self {
        let text = |key: &str| m[key].as_str().unwrap_or_default().to_owned();
        let effect = text("effect");
        Self {
            name: text("name"),
            description: text("description"),
            idempotent: m["idempotent"].as_bool().unwrap_or(false),
            parallel_safe: m["parallel_safe"].as_bool().unwrap_or(false),
            // The runtime gates destructive tools as if they required confirmation.
            confirmation: m["confirmation"] == "required" || effect == "destructive",
            input: schema::fields(&m["input_schema"]),
            output: schema::fields(&m["output_schema"]),
            skeleton: schema::skeleton(&m["input_schema"]),
            effect,
        }
    }
}

/// A single-line text field with a cursor, counted in characters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Input {
    pub text: String,
    pub cursor: usize,
}

impl Input {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }
    fn byte(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }
    fn edit(&mut self, key: KeyEvent) -> bool {
        let len = self.text.chars().count();
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                *self = Self::default()
            }
            KeyCode::Char(c) => {
                let at = self.byte(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Delete if self.cursor < len => {
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tools,
    Arguments,
    RequestId,
    Result,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    Help,
    /// Grant (`true`) or revoke `confirm:<tool>`.
    Confirm {
        tool: String,
        grant: bool,
    },
    History {
        selected: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Starting,
    Ready,
    Failed(String),
}

/// One finished call, as shown in the result pane and the history.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub tool: String,
    pub arguments: String,
    pub request_id: String,
    pub status: String,
    pub duration_ms: f64,
    pub replayed: bool,
    pub body: Value,
    pub error_code: Option<String>,
    pub suggested_action: Option<String>,
}

#[derive(Debug)]
enum Pending {
    Tools,
    Call {
        tool: String,
        arguments: String,
        request_id: String,
    },
    Confirm {
        tool: String,
    },
}

pub struct App {
    pub phase: Phase,
    pub server: String,
    pub tools: Vec<ToolInfo>,
    pub selected: usize,
    pub focus: Focus,
    pub arguments: HashMap<String, Input>,
    /// Per tool: reusing one request_id across tools would be an idempotency conflict.
    pub request_ids: HashMap<String, Input>,
    pub confirmed: BTreeSet<String>,
    pub modal: Option<Modal>,
    pub result: Option<Call>,
    pub history: Vec<Call>,
    pub scroll: u16,
    pub status: String,
    pub logs: Vec<String>,
    pub quit: bool,
    pending: HashMap<u64, Pending>,
    next_id: u64,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            phase: Phase::Starting,
            server: String::new(),
            tools: Vec::new(),
            selected: 0,
            focus: Focus::Tools,
            arguments: HashMap::new(),
            request_ids: HashMap::new(),
            confirmed: BTreeSet::new(),
            modal: None,
            result: None,
            history: Vec::new(),
            scroll: 0,
            status: "Building the application…".into(),
            logs: Vec::new(),
            quit: false,
            pending: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn tool(&self) -> Option<&ToolInfo> {
        self.tools.get(self.selected)
    }

    /// The arguments field of the selected tool, prefilled from its input schema.
    pub fn arguments(&self) -> Input {
        self.tool()
            .map(|t| {
                self.arguments
                    .get(&t.name)
                    .cloned()
                    .unwrap_or_else(|| Input::new(t.skeleton.clone()))
            })
            .unwrap_or_default()
    }

    /// The request_id field of the selected tool.
    pub fn request_id(&self) -> Input {
        self.tool()
            .and_then(|t| self.request_ids.get(&t.name).cloned())
            .unwrap_or_default()
    }

    fn request_id_mut(&mut self) -> Option<&mut Input> {
        let name = self.tool()?.name.clone();
        Some(self.request_ids.entry(name).or_default())
    }

    fn arguments_mut(&mut self) -> Option<&mut Input> {
        let tool = self.tool()?.clone();
        Some(
            self.arguments
                .entry(tool.name)
                .or_insert_with(|| Input::new(tool.skeleton)),
        )
    }

    pub fn locked(&self, tool: &ToolInfo) -> bool {
        tool.confirmation && !self.confirmed.contains(&tool.name)
    }

    fn request(&mut self, pending: Pending, mut body: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        body["id"] = id.into();
        self.pending.insert(id, pending);
        body
    }

    fn run(&mut self, tool: &str, arguments: &str, request_id: &str) -> Vec<Value> {
        let parsed: Value = match serde_json::from_str(arguments) {
            Ok(value) => value,
            Err(e) => {
                self.status = format!("Arguments are not valid JSON: {e}");
                return vec![];
            }
        };
        let mut body = json!({ "op": "call", "tool": tool, "arguments": parsed });
        if !request_id.trim().is_empty() {
            body["request_id"] = request_id.trim().into();
        }
        self.status = format!("Running {tool}…");
        let pending = Pending::Call {
            tool: tool.to_owned(),
            arguments: arguments.to_owned(),
            request_id: request_id.trim().to_owned(),
        };
        vec![self.request(pending, body)]
    }

    fn run_selected(&mut self) -> Vec<Value> {
        let Some(tool) = self.tool().map(|t| t.name.clone()) else {
            return vec![];
        };
        let arguments = self.arguments().text;
        let request_id = self.request_id().text;
        self.run(&tool, &arguments, &request_id)
    }

    /// Handles a key and returns the protocol requests it triggers.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Value> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return vec![];
        }
        if let Some(modal) = self.modal.clone() {
            return self.handle_modal(modal, key);
        }
        if self.phase != Phase::Ready {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                self.quit = true;
            }
            return vec![];
        }
        let editing = matches!(self.focus, Focus::Arguments | Focus::RequestId);
        match key.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Tools => Focus::Arguments,
                    Focus::Arguments => Focus::RequestId,
                    Focus::RequestId => Focus::Result,
                    Focus::Result => Focus::Tools,
                };
                return vec![];
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Tools => Focus::Result,
                    Focus::Arguments => Focus::Tools,
                    Focus::RequestId => Focus::Arguments,
                    Focus::Result => Focus::RequestId,
                };
                return vec![];
            }
            KeyCode::Esc if editing => {
                self.focus = Focus::Tools;
                return vec![];
            }
            KeyCode::Enter if editing => return self.run_selected(),
            _ => {}
        }
        match self.focus {
            Focus::Arguments => {
                if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    if let Some(tool) = self.tool().cloned() {
                        self.arguments.insert(tool.name, Input::new(tool.skeleton));
                    }
                } else if let Some(input) = self.arguments_mut() {
                    input.edit(key);
                }
                vec![]
            }
            Focus::RequestId => {
                if let Some(input) = self.request_id_mut() {
                    input.edit(key);
                }
                vec![]
            }
            Focus::Tools | Focus::Result => self.handle_command(key),
        }
    }

    fn handle_command(&mut self, key: KeyEvent) -> Vec<Value> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('?') => self.modal = Some(Modal::Help),
            KeyCode::Char('h') => {
                if !self.history.is_empty() {
                    self.modal = Some(Modal::History {
                        selected: self.history.len() - 1,
                    });
                }
            }
            KeyCode::Char('r') => {
                if let Some(last) = self.history.last().cloned() {
                    return self.run(&last.tool, &last.arguments, &last.request_id);
                }
            }
            KeyCode::Char('c') => {
                if let Some(tool) = self.tool().cloned() {
                    if tool.confirmation {
                        let grant = !self.confirmed.contains(&tool.name);
                        self.modal = Some(Modal::Confirm {
                            tool: tool.name,
                            grant,
                        });
                    } else {
                        self.status = format!("{} doesn't require confirmation", tool.name);
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') if self.focus == Focus::Tools => {
                self.selected = self.selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') if self.focus == Focus::Tools => {
                self.selected = (self.selected + 1).min(self.tools.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Enter if self.focus == Focus::Tools => self.focus = Focus::Arguments,
            _ => {}
        }
        vec![]
    }

    fn handle_modal(&mut self, modal: Modal, key: KeyEvent) -> Vec<Value> {
        match modal {
            Modal::Help => {
                self.modal = None;
                vec![]
            }
            Modal::Confirm { tool, grant } => {
                self.modal = None;
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    let op = if grant { "confirm" } else { "revoke" };
                    return vec![self.request(
                        Pending::Confirm { tool: tool.clone() },
                        json!({ "op": op, "tool": tool }),
                    )];
                }
                vec![]
            }
            Modal::History { selected } => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.modal = Some(Modal::History {
                            selected: selected.saturating_sub(1),
                        })
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let last = self.history.len().saturating_sub(1);
                        self.modal = Some(Modal::History {
                            selected: (selected + 1).min(last),
                        })
                    }
                    KeyCode::Enter => {
                        self.modal = None;
                        if let Some(call) = self.history.get(selected).cloned() {
                            return self.run(&call.tool, &call.arguments, &call.request_id);
                        }
                    }
                    _ => self.modal = None,
                }
                vec![]
            }
        }
    }

    /// Applies a protocol line; returns follow-up requests (the catalog after `ready`).
    pub fn apply(&mut self, line: Value) -> Vec<Value> {
        if line["event"] == "ready" {
            self.phase = Phase::Ready;
            self.server = line["server"].as_str().unwrap_or("carmy").to_owned();
            self.status = format!("Connected to {} · press ? for help", self.server);
            return vec![self.request(Pending::Tools, json!({ "op": "tools" }))];
        }
        let Some(pending) = line["id"].as_u64().and_then(|id| self.pending.remove(&id)) else {
            return vec![];
        };
        if line["ok"] != true {
            let error = &line["error"];
            self.status = format!(
                "{}: {}",
                error["code"].as_str().unwrap_or("ERROR"),
                error["message"].as_str().unwrap_or_default()
            );
            return vec![];
        }
        let result = &line["result"];
        match pending {
            Pending::Tools => {
                self.tools = result["tools"]
                    .as_array()
                    .map(|tools| tools.iter().map(ToolInfo::from_metadata).collect())
                    .unwrap_or_default();
                self.selected = self.selected.min(self.tools.len().saturating_sub(1));
            }
            Pending::Confirm { tool } => {
                if result["confirmed"] == true {
                    self.confirmed.insert(tool.clone());
                    self.status = format!("confirm:{tool} granted for this session");
                } else {
                    self.confirmed.remove(&tool);
                    self.status = format!("confirm:{tool} revoked");
                }
            }
            Pending::Call {
                tool,
                arguments,
                request_id,
            } => {
                let error = result.get("error");
                let call = Call {
                    status: result["status"].as_str().unwrap_or("unknown").to_owned(),
                    duration_ms: result["duration_ms"].as_f64().unwrap_or(0.0),
                    replayed: result["replayed"] == true,
                    body: error
                        .or_else(|| result.get("data"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    error_code: error.and_then(|e| e["code"].as_str()).map(str::to_owned),
                    suggested_action: error
                        .and_then(|e| e["suggested_action"].as_str())
                        .map(str::to_owned),
                    tool,
                    arguments,
                    request_id,
                };
                self.status = format!(
                    "{} {} in {:.1} ms",
                    call.tool, call.status, call.duration_ms
                );
                self.scroll = 0;
                self.focus = Focus::Result;
                self.history.push(call.clone());
                self.result = Some(call);
            }
        }
        vec![]
    }

    pub fn log(&mut self, line: String) {
        self.logs.push(line);
        if self.logs.len() > 200 {
            self.logs.remove(0);
        }
    }

    /// The application exited; before `ready` that means the build or startup failed.
    pub fn exited(&mut self, code: Option<i32>) {
        if self.phase != Phase::Ready {
            let code = code.map_or("unknown".into(), |c| c.to_string());
            self.phase = Phase::Failed(format!("the application exited (code {code})"));
        } else {
            self.phase = Phase::Failed("the application exited".into());
        }
    }
}
