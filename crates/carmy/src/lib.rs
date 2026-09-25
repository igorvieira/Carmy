//! Agent-native execution infrastructure for Rust.
pub use carmy_core::*;
pub use carmy_macros::tool;
pub use carmy_schema::schema;
pub mod prelude {
    pub use crate::{AgentContext, AgentError, AgentResult, Effect, Tool};
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};
}
