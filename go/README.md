# smooth-operator — Go client

An idiomatic, transport-agnostic Go client for the [smooth-operator](../docs/PROTOCOL.md)
WebSocket protocol, generated from the JSON Schemas in [`../spec/`](../spec).

```bash
go get github.com/SmooAI/smooth-operator/go/protocol
```

## Usage

```go
package main

import (
	"context"
	"fmt"

	"github.com/SmooAI/smooth-operator/go/protocol"
)

func main() {
	ctx := context.Background()

	c, _ := protocol.New(protocol.Options{
		Transport: protocol.NewWebSocketTransport("wss://realtime.prod.smooth-agent.dev", nil),
	})
	_ = c.Connect(ctx)
	defer c.Close()

	sess, _ := c.CreateConversationSession(ctx, protocol.CreateConversationSessionParams{
		AgentID:  "11111111-1111-1111-1111-111111111111",
		UserName: "Alice",
	})

	turn := c.SendMessage(protocol.SendMessageParams{SessionID: sess.SessionID, Message: "Where's my order?"})
	for ev := range turn.Events() {
		switch ev.Type {
		case protocol.EventStreamToken:
			tok, _ := ev.AsStreamToken()
			fmt.Print(tok.Data.Token)
		case protocol.EventWriteConfirmationRequired:
			// HITL: approve and the resumed stream flows back into this same turn.
			c.ConfirmToolAction(protocol.ConfirmToolActionParams{
				SessionID: sess.SessionID, RequestID: turn.RequestID(), Approved: true,
			})
		}
	}
	final, _ := turn.Wait(ctx)
	fmt.Println("\nmessageId:", final.Data.Data.MessageID)
}
```

## Layout

| File                  | Purpose                                                                 |
| --------------------- | ----------------------------------------------------------------------- |
| `protocol/types_gen.go` | Generated wire types (one struct per schema / `$def`). **Do not edit.** |
| `protocol/events.go`  | Ergonomic `ServerEvent` discrimination + typed `As*` accessors.         |
| `protocol/transport.go` | `Transport` interface + default `coder/websocket` implementation.     |
| `protocol/client.go`  | `Client` with the action methods.                                       |
| `protocol/turn.go`    | `MessageTurn` (streaming events + awaitable terminal) and `ProtocolError`. |
| `protocol/validate.go` | Optional runtime JSON Schema validation against `../spec/`.            |

## Discriminating events

Go has no sum types. A `ServerEvent` carries the common envelope fields (`Type`,
`RequestID`, `Status`, `Node`, `Token`) plus the raw frame bytes. Switch on `Type`
and call the matching `As*` accessor to decode the concrete generated payload:

```go
switch ev.Type {
case protocol.EventEventualResponse:
	final, _ := ev.AsEventualResponse()
	// final.Data.Data.MessageID  ← note the protocol's nested data.data
case protocol.EventError:
	errEv, _ := ev.AsError()
	...
}
```

## Regenerating types

Types are generated with [`go-jsonschema`](https://github.com/atombender/go-jsonschema)
(pure Go, works offline — preferred over `quicktype` for the Go target):

```bash
go install github.com/atombender/go-jsonschema@latest

go-jsonschema -p protocol --only-models --tags json -t \
  --capitalization ID --capitalization OTP --capitalization UUID \
  --capitalization HMAC --capitalization JSON --capitalization URL \
  spec/domain/*.schema.json spec/actions/*.schema.json spec/events/*.schema.json spec/envelope.schema.json \
  -o go/protocol/types_gen.go
```

`--only-models` is intentional: it emits plain structs with no generated
`UnmarshalJSON` enum validation, so the client tolerates forward-compatible wire
values and the conformance fixtures round-trip cleanly.
