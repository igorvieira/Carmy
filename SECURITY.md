# Security policy

Carmy sits between AI agents and systems with real side effects, so security reports
matter to us.

## Reporting a vulnerability

Please **do not open a public issue**. Report privately through GitHub:
**[Security → Report a vulnerability](https://github.com/igorvieira/Carmy/security/advisories/new)**.

Include what you found, how to reproduce it, and the impact you expect. You will get an
acknowledgement within 72 hours, and we will keep you informed until a fix is released.
With your permission, you will be credited in the advisory.

## Supported versions

Carmy is pre-1.0. Security fixes land in the latest `0.x` release.

## Scope

In scope, among others:

- bypassing effect or confirmation policies
- a replay or idempotency flaw that repeats a side effect
- context or permission injection through requests
- leaking tool arguments or outputs through logs
- denial of service through the HTTP or MCP adapters
