# Contributing

Thanks for helping build Carmy.

## Checks

These are the checks CI runs, on stable Rust:

```console
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo bench --workspace --no-run
cargo test -p carmy-cli --test new -- --ignored   # builds a generated application
```

Macro diagnostics are snapshot-tested with `trybuild` (`crates/carmy/tests/ui`). After an
intentional change to an error message, regenerate the snapshots with
`TRYBUILD=overwrite cargo test -p carmy`.

## Design rules

- Keep `carmy-core` free of transports, and keep HTTP and MCP concepts in their adapters.
- Keep application state out of `AgentContext`.
- Add abstractions only when they are needed, and prefer small hooks to frameworks.
- Accompany a behavior change with a test of the contract or failure mode it affects.
- Do not add performance claims that `cargo bench` cannot reproduce.

See the [architecture review checklist](https://carmy-pi.vercel.app/reference/architecture/#review-checklist).
The documentation website lives in `website/` (`pnpm install && pnpm dev`).

## Commits

Use [conventional commits](https://www.conventionalcommits.org/) (`feat(runtime): …`,
`fix(http): …`, `docs: …`). APIs are unstable during 0.x.
