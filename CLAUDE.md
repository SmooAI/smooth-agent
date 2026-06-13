# CLAUDE.md — smooth-operator

Guidance for Claude Code (and humans) working in this repo.

## The mission: every language to FULL parity with the Rust reference, through TDD

smooth-operator is **polyglot**. The Rust implementation (`smooai-smooth-operator-core` +
`rust/smooth-operator-server` + `rust/adapters/*`) is the **reference**. Every other language
— its **clients**, its **engine cores** (the [[Polyglot Cores]] track: C#, then Python/TS), and
its **servers** — is built to **full behavioral parity with Rust**, and we get there
**test-first (TDD)**.

This is the standing goal. When you add or extend any language implementation:

1. **Find the Rust behavior + its test.** Every capability in Rust has a test (a unit test, a
   conformance check, an integration test like `rust/smooth-operator-server/tests/protocol_smoke.rs`,
   or an eval scenario in `rust/evals`).
2. **Write the parity test first**, in that language's idiom (xUnit for .NET, etc.), asserting the
   **same behavior** — same event shapes, same sequence, same edge cases. Watch it fail.
3. **Implement until it's green.** Parity is enforced by tests, not by mirroring type shapes —
   each language uses its own idioms (the C# core is built on `Microsoft.Extensions.AI`, etc.).
4. **Keep the test named/scoped to its Rust counterpart** so parity gaps are visible.

A feature isn't "done" in a language until its parity tests are green in that language.

## The shared contract (what "parity" is checked against)

- **Protocol** — the language-neutral JSON Schemas in `spec/`, with canonical instances in
  `spec/conformance/fixtures.json`. Every client/server validates its frames against these
  (e.g. the .NET `ProtocolValidator`; the C# server's `ServerProtocolTests` validate produced
  events against the same schemas).
- **Behavioral parity tests** — port each Rust unit/integration test (e.g. the server's
  `ping_returns_pong`, `create_session_returns_valid_descriptor`, `eventual_response_*`,
  `unknown_action_errors_without_dropping_connection`) as the language's own test.
- **Eval scenarios** — the five `rust/evals` scenarios, judged ≥ 4.0, run (gated) per language.

## Layers (don't conflate them)

| Layer | What | Rust | C# |
| --- | --- | --- | --- |
| Engine | the generic agent framework | `smooai-smooth-operator-core` | `SmooAI.SmoothOperator.Core` |
| Service | the system on the engine (protocol host, storage, ingestion, ACL, auth) | `smooth-operator-server` | `SmooAI.SmoothOperator.Server` (+ `.AspNetCore`) |
| Client | talks to a running service | the polyglot clients | `SmooAI.SmoothOperator` |

See `docs/Architecture/Polyglot Cores.md` for the engine + server roadmaps and the parity
contract in full.

## Tests must run and pass

- Unit/parity/conformance tests run in CI with **no credentials** and must be green.
- Live tests (real gateway / LLM judge) are **gated** (`SMOOTH_AGENT_E2E=1` + `SMOOAI_GATEWAY_KEY`)
  and **skip cleanly** when absent — never fail for missing creds.
- Don't land a language change without its parity tests.

## Releases

All published artifacts ship in **lockstep** via Changesets (`pnpm changeset` → merge → the
Release workflow versions + publishes npm/NuGet/… together). Don't hand-edit versions; see
`.changeset/README.md`.
