# `@smooai/smooth-operator-agent`

TypeScript protocol types and a native WebSocket client for the **smooth-operator-agent**
protocol. This is the first polyglot client; it sets the pattern for the Go, .NET,
and Python clients.

The wire contract is the language-neutral JSON Schema in [`../spec`](../spec). The
TypeScript types are **generated** from those schemas (and committed, so consumers
don't need the generator), with an ergonomic layer (discriminated unions + guards)
on top.

## Install

```bash
pnpm add @smooai/smooth-operator-agent
```

Requires Node ≥ 22, ESM only.

## Usage

```ts
import { SmoothAgentClient } from '@smooai/smooth-operator-agent';

const client = new SmoothAgentClient({ url: 'wss://realtime.example.dev' });
await client.connect();

const session = await client.createConversationSession({ agentId, userName: 'Alice' });

// Streaming turn: await for the final response, or async-iterate the events.
const turn = client.sendMessage({ sessionId: session.sessionId, message: 'Where is my order?' });

for await (const ev of turn) {
    if (ev.type === 'stream_token') process.stdout.write(ev.token ?? '');
    if (ev.type === 'write_confirmation_required') {
        client.confirmToolAction({ sessionId: session.sessionId, requestId: turn.requestId, approved: true });
    }
}

const final = await turn; // EventualResponse — the authoritative terminal state
```

### Transport injection

The client never touches a real socket directly — it talks to an injectable
`Transport`. The default uses the global `WebSocket`. On Node, inject the `ws`
package; in tests, inject a mock:

```ts
import WebSocket from 'ws';
new SmoothAgentClient({ url, webSocketFactory: (u) => new WebSocket(u) });
```

### Runtime validation (optional)

`ProtocolValidator` compiles the spec schemas with ajv and validates frames:

```ts
import { ProtocolValidator } from '@smooai/smooth-operator-agent';
const v = await ProtocolValidator.load();
v.validateEvent(incomingEvent); // { valid, errors }
```

This reads schema files from disk, so it's Node-only — the wire client does not
depend on it.

## Scripts

| Script           | Purpose                                                  |
| ---------------- | -------------------------------------------------------- |
| `pnpm generate`  | Regenerate `src/generated/types.ts` from `../spec`.      |
| `pnpm build`     | `tsc` → `dist/`.                                          |
| `pnpm typecheck` | Type-check `src/` and `test/` without emitting.          |
| `pnpm test`      | Vitest (conformance fixtures + client + type-level).     |

## Codegen

`scripts/generate.ts` reads every `*.schema.json` under `../spec` and emits a
single `src/generated/types.ts` via `json-schema-to-typescript`.

Every schema in the spec is self-contained — all `$ref`s point at internal
`#/$defs/...` definitions, with **no cross-file refs** — so each file compiles
independently. Files whose top level is a `oneOf` over `$defs` (the envelope and
the action files) are expanded so each named `$def` (`Request`, `Response`, …)
gets its own exported interface; flat event/domain files compile as-is. Shared
definitions that several files declare (e.g. `ErrorObject`, `ConversationMessage`)
are deduplicated by name during concatenation.

The generated file is committed; CI can `pnpm generate` and `git diff --exit-code`
to catch schemas that changed without a regenerate.
