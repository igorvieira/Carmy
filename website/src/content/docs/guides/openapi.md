---
title: OpenAPI
description: "Turn an existing API into tools with effects, confirmation and replay-safe retries."
sidebar:
  order: 19
---

An API you already have becomes tools an agent can use safely. `carmy::openapi`
(feature `openapi`) reads an OpenAPI 3.0 or 3.1 description and makes one tool per
operation, each with an effect, a confirmation rule, a validated input schema and
replay-safe retries, before any agent touches the API.

```toml
carmy = { version = "0.6", features = ["openapi"] }
```

```rust
let github = carmy::openapi::OpenApi::from_file("github.json")?
    .base_url("https://api.github.com")
    .header("Authorization", format!("Bearer {}", std::env::var("GITHUB_TOKEN")?))
    .only(["issues/list-for-repo", "issues/create"])
    .prefix("github_")
    .tools()?;

carmy::app().tools(github).run().await
```

## What each operation becomes

| from the description | in the tool |
|----------------------|-------------|
| `operationId` | the name (with `.prefix(..)`; characters Carmy does not accept become `_`) |
| `summary`, `description` | the description agents read |
| path and query parameters | input properties; path parameters are required |
| the JSON request body | the `body` input property |
| referenced component schemas | the tool's own `$defs`, only what it uses |
| OpenAPI 3.0 `nullable: true` | a type that admits `null` |

Header and cookie parameters are left out: they carry credentials and plumbing, which
the host sets with `.header(..)`, never the agent.

The method sets the effect:

| method | effect | idempotent | confirmation |
|--------|--------|------------|--------------|
| `GET`, `HEAD` | `read` | yes | no |
| `PUT` | `external_write` | yes | no |
| `POST`, `PATCH` | `external_write` | no | no |
| `DELETE` | `destructive` | yes | required |

An operation that reads with `POST` (a search, say) says so with `x-carmy-effect: read`
in the description, or `.effect("searchPets", Effect::Read)` in code.

## Calls

- The input is validated against the schema before anything leaves the process.
- Path values are percent-encoded, `/` included, so they never leave their segment.
- A call's `request_id` travels as the `Idempotency-Key` header on writes, so an API
  that honors it dedupes retries too.
- The host's headers go on every call; agents never see them.

The API's answers become [errors](/guides/errors/) agents can act on:

| API answer | code | retry |
|------------|------|-------|
| 429 | `UPSTREAM_RATE_LIMITED` | yes, after `Retry-After` |
| 503, other 5xx | `UPSTREAM_UNAVAILABLE` | yes |
| 401, 403 | `UPSTREAM_UNAUTHORIZED` | no |
| 404 | `UPSTREAM_NOT_FOUND` | no |
| 409 | `UPSTREAM_CONFLICT` | no |
| other 4xx | `UPSTREAM_REJECTED` | no |
| no connection | `UPSTREAM_UNREACHABLE` | yes: the request never left |
| no answer | `UPSTREAM_FAILED` | only if the tool is idempotent |

`details` carries the status and the body (the first 2 KiB of a text body).

## Responses

Responses are not validated against the description by default: APIs drift from their
documents, and a new field should not fail a call that succeeded. `.strict_output(true)`
validates them against the 2xx schemas, and gives agents those schemas.

## Options

| method | does |
|--------|------|
| `.base_url(url)` | where the API lives; default: the first `servers` URL |
| `.header(name, value)` | a header on every call |
| `.only([..])` | only these `operationId`s |
| `.prefix(p)` | prepended to every tool name |
| `.effect(id, effect)` | override one operation's effect |
| `.strict_output(true)` | validate responses |
| `.client(reqwest::Client)` | proxies, timeouts, TLS settings |

JSON descriptions only for now; convert YAML with any tool first.
