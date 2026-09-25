//! JSON Schema generation shared by tools and discovery.
pub fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
    schemars::schema_for!(T).to_value()
}
#[cfg(test)]
mod tests {
    #[test]
    fn schema_contains_constraints() {
        assert_eq!(super::schema::<String>()["type"], "string");
    }
}
