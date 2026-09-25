//! Application state injected into tools, kept apart from the framework's `AgentContext`.
use crate::{AgentError, AgentResult, ErrorCategory, Tool};
use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
    sync::Arc,
};

/// A dependency registered with [`Carmy::state`](crate::Carmy::state) and received as a
/// tool argument: `async fn create_order(State(db): State<Db>, input: Input)`.
///
/// Each execution receives a clone, so share expensive values through `Arc` or use
/// types that are already cheap to clone (connection pools, HTTP clients).
#[derive(Debug, Clone, Copy, Default)]
pub struct State<T>(pub T);

/// Application dependencies, keyed by type.
#[derive(Default, Clone)]
pub struct StateMap(HashMap<TypeId, Arc<dyn Any + Send + Sync>>);
impl StateMap {
    pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Arc::new(value));
    }
    /// Resolve a dependency of `tool`; a missing type is a startup error, not a request error.
    pub fn get<T: Clone + 'static>(&self, tool: &str) -> AgentResult<T> {
        self.0
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
            .ok_or_else(|| {
                AgentError::new(
                    "MISSING_STATE",
                    format!(
                        "tool `{tool}` requires State<{}>; register it with .state(..)",
                        type_name::<T>()
                    ),
                    ErrorCategory::Internal,
                )
            })
    }
}

/// Anything that becomes a [`Tool`] once application state is available.
/// Every `Tool` qualifies; `#[carmy::tool]` implements it for tools taking `State<T>`.
pub trait IntoTool {
    type Tool: Tool;
    fn into_tool(self, states: &StateMap) -> AgentResult<Self::Tool>;
}
impl<T: Tool> IntoTool for T {
    type Tool = T;
    fn into_tool(self, _: &StateMap) -> AgentResult<T> {
        Ok(self)
    }
}
