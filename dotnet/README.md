# `SmooAI.SmoothOperatorAgent`

C#/.NET protocol types and a native WebSocket client for the **smooth-operator-agent**
protocol. Sibling to the [TypeScript](../typescript) reference client.

The wire contract is the language-neutral JSON Schema in [`../spec`](../spec). The
C# types are **generated** from those schemas with [NJsonSchema](https://github.com/RicoSuter/NJsonSchema)
(and committed, so consumers don't need the generator), with an ergonomic layer
(discriminated unions over System.Text.Json polymorphism) on top.

## Target framework

`net8.0`. The .NET 10 SDK ships the .NET 8 targeting pack, so the library builds
and tests run on `net8.0` for the broadest consumer reach.

## Layout

| Path | What |
| --- | --- |
| `src/Generated/Types.cs` | **Generated** 1:1 reflection of the JSON Schemas (committed). |
| `src/Types.cs` | Ergonomic `ServerEvent` / `ClientAction` discriminated unions + enums. |
| `src/Transport.cs` | `ITransport` (mockable) + `WebSocketTransport` (`ClientWebSocket`). |
| `src/SmoothAgentClient.cs` | Typed async client; `MessageTurn` streaming. |
| `src/ProtocolValidator.cs` | Runtime validation against the spec schemas. |
| `src/EnumMemberStringConverter.cs` | STJ enum converter that honors `[EnumMember]` wire values. |
| `tools/Generator/` | Codegen tool (`dotnet run --project tools/Generator`). |
| `tests/` | xUnit conformance + client + serialization tests. |

## Usage

```csharp
using SmooAI.SmoothOperatorAgent;

await using var client = new SmoothAgentClient(new SmoothAgentClientOptions
{
    Url = "wss://realtime.prod.smooth-agent.dev",
});
await client.ConnectAsync();

var session = await client.CreateConversationSessionAsync(
    new CreateConversationSessionAction { AgentId = agentId, UserName = "Alice" });

// Streaming turn: iterate intermediate events AND await the terminal response.
var turn = client.SendMessageAsync(new SendMessageAction
{
    SessionId = session.SessionId,
    Message = "What is the status of my last order?",
    Stream = true,
});

await foreach (var ev in turn)
{
    if (ev is StreamTokenEvent t) Console.Write(t.Token);
}

EventualResponseEvent final = await turn.Completion;
Console.WriteLine($"\nmessageId: {final.Data.Payload.MessageId}");
```

### HITL resume

`write_confirmation_required` / `otp_verification_required` events arrive on the
originating turn (same `requestId`). Reply with the matching action; the resumed
stream flows back into the same `MessageTurn`:

```csharp
await client.ConfirmToolActionAsync(sessionId, turn.RequestId, approved: true);
await client.VerifyOtpAsync(sessionId, turn.RequestId, code: "123456");
```

### Runtime validation

```csharp
var validator = await ProtocolValidator.LoadAsync(); // walks up to find spec/
var result = validator.ValidateAction(ActionTypes.SendMessage, frameJson);
if (!result.IsValid) Console.Error.WriteLine(result.FormatErrors());
```

> Note: NJsonSchema 11.x does not enforce JSON Schema `const`, so the validator
> catches structural violations (`required`, `additionalProperties: false`, types)
> but not a wrong discriminator value. The discriminated-union deserialization in
> `src/Types.cs` is what enforces the `type` / `action` discriminator at runtime.

## Regenerating types

After a schema change in `../spec`:

```bash
dotnet run --project tools/Generator
```

## Build & test

```bash
dotnet build SmooAI.SmoothOperatorAgent.slnx
dotnet test  SmooAI.SmoothOperatorAgent.slnx
```
