# smooth-agent (Python)

Python protocol types and a native **async** WebSocket client for the
[smooth-agent](../docs/PROTOCOL.md) protocol. The wire contract lives in the
language-neutral JSON Schemas under [`../spec/`](../spec); the pydantic models in
`smooth_agent._generated` are generated from those schemas and committed.

This mirrors the structure/ergonomics of the TypeScript reference client in
[`../typescript/`](../typescript).

## Install / develop

```bash
uv sync
uv run python -c "import smooth_agent"
uv run pytest
```

## Regenerate models from the spec

```bash
uv run python scripts/generate.py
```

This reads `../spec/**/*.schema.json` and writes
`src/smooth_agent/_generated.py` using
[datamodel-code-generator](https://github.com/koxudaxi/datamodel-code-generator).

## Usage

```python
import asyncio
from smooth_agent import SmoothAgentClient

async def main():
    client = SmoothAgentClient(url="wss://realtime.prod.smooth-agent.dev")
    await client.connect()

    session = await client.create_conversation_session(agent_id="…")

    turn = client.send_message(session_id=session.session_id, message="Hello!")
    async for event in turn:                # stream_token / stream_chunk / HITL events
        if event.type == "stream_token":
            print(event.token, end="", flush=True)
    final = await turn                       # the terminal eventual_response

asyncio.run(main())
```

## Naming: camelCase wire, snake_case Python

The JSON wire format is camelCase (`requestId`, `sessionId`). The pydantic models
use **snake_case Python attributes** with camelCase **aliases**, and
`populate_by_name = True`, so:

- You construct/access models with idiomatic snake_case (`session.session_id`).
- Serialization with `model_dump(by_alias=True)` / `model_dump_json(by_alias=True)`
  emits the camelCase wire form.
