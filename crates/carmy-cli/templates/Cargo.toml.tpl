[package]
name = "{{name}}"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
carmy = {{carmy}}
serde = { version = "1", features = ["derive"] }
schemars = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
