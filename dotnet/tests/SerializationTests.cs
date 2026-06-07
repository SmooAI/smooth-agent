// (De)serialization round-trip tests for the generated domain types. These pin the
// wire enum values that carry kebab/snake casing (Participant.type = "ai-agent",
// Message.direction = "inbound", GeneralAgentResponse.resolutionStatus = "in_progress")
// and the Session.threadId field — all of which a naive enum converter would mangle.

using System.Text.Json;
using SmooAI.SmoothAgent.Generated;

namespace SmooAI.SmoothAgent.Tests;

public sealed class SerializationTests
{
    private static readonly JsonSerializerOptions Options =
        new(JsonSerializerDefaults.Web) { WriteIndented = false };

    [Fact]
    public void Participant_AiAgent_RoundTrips_WithKebabWireValue()
    {
        const string json = """
        {
          "id": "11111111-1111-1111-1111-111111111111",
          "conversationId": "22222222-2222-2222-2222-222222222222",
          "organizationId": "33333333-3333-3333-3333-333333333333",
          "type": "ai-agent",
          "name": "Aria",
          "createdAt": "2026-06-07T00:00:00Z",
          "updatedAt": "2026-06-07T00:00:00Z"
        }
        """;

        var participant = JsonSerializer.Deserialize<Participant>(json, Options)!;
        Assert.Equal(ParticipantType.AiAgent, participant.Type);
        Assert.Equal("Aria", participant.Name);

        // Re-serialize and confirm the wire value is the kebab-cased "ai-agent".
        var roundTripped = JsonSerializer.Serialize(participant, Options);
        using var doc = JsonDocument.Parse(roundTripped);
        Assert.Equal("ai-agent", doc.RootElement.GetProperty("type").GetString());
    }

    [Fact]
    public void Message_Direction_Inbound_RoundTrips()
    {
        const string json = """
        {
          "id": "44444444-4444-4444-4444-444444444444",
          "direction": "inbound",
          "content": { "text": "What is the status of my order?" },
          "createdAt": "2026-06-07T00:00:00Z"
        }
        """;

        var message = JsonSerializer.Deserialize<Message>(json, Options)!;
        Assert.Equal(MessageDirection.Inbound, message.Direction);

        var roundTripped = JsonSerializer.Serialize(message, Options);
        using var doc = JsonDocument.Parse(roundTripped);
        Assert.Equal("inbound", doc.RootElement.GetProperty("direction").GetString());
    }

    [Fact]
    public void Session_ThreadId_AndStatus_RoundTrip()
    {
        const string json = """
        {
          "sessionId": "55555555-5555-5555-5555-555555555555",
          "conversationId": "66666666-6666-6666-6666-666666666666",
          "agentId": "77777777-7777-7777-7777-777777777777",
          "agentName": "Aria",
          "userParticipantId": "88888888-8888-8888-8888-888888888888",
          "agentParticipantId": "99999999-9999-9999-9999-999999999999",
          "threadId": "thread-abc-123",
          "status": "active"
        }
        """;

        var session = JsonSerializer.Deserialize<Session>(json, Options)!;
        Assert.Equal("thread-abc-123", session.ThreadId);
        Assert.Equal(SessionStatus.Active, session.Status);

        var roundTripped = JsonSerializer.Serialize(session, Options);
        using var doc = JsonDocument.Parse(roundTripped);
        Assert.Equal("thread-abc-123", doc.RootElement.GetProperty("threadId").GetString());
        Assert.Equal("active", doc.RootElement.GetProperty("status").GetString());
    }

    [Fact]
    public void GeneralAgentResponse_SnakeCasedResolutionStatus_RoundTrips()
    {
        const string json = """
        {
          "responseParts": ["Working on it."],
          "customerHappinessScore": 0.8,
          "needsSatisfactionScore": 0.7,
          "requestSummary": "User asked for an update.",
          "resolutionStatus": "in_progress",
          "suggestedNextActions": ["Wait for confirmation"]
        }
        """;

        var response = JsonSerializer.Deserialize<GeneralAgentResponse>(json, Options)!;
        Assert.Equal(GeneralAgentResponseResolutionStatus.In_progress, response.ResolutionStatus);

        var roundTripped = JsonSerializer.Serialize(response, Options);
        using var doc = JsonDocument.Parse(roundTripped);
        Assert.Equal("in_progress", doc.RootElement.GetProperty("resolutionStatus").GetString());
    }

    [Fact]
    public void ServerEvent_DeserializesPolymorphically_ByType()
    {
        const string json = """
        {"type":"stream_token","requestId":"req-1","token":"Hi","data":{"requestId":"req-1","token":"Hi"}}
        """;

        var ev = JsonSerializer.Deserialize<ServerEvent>(json, Options)!;
        var token = Assert.IsType<StreamTokenEvent>(ev);
        Assert.Equal("Hi", token.Token);
        Assert.Equal("req-1", token.RequestId);
        Assert.Equal("Hi", token.Data.Token);
    }

    [Fact]
    public void ClientAction_SerializesWithActionDiscriminator()
    {
        var action = new SendMessageAction
        {
            RequestId = "req-2",
            SessionId = "sess-1",
            Message = "hello",
            Stream = true,
        };

        var json = JsonSerializer.Serialize<ClientAction>(action, Options);
        using var doc = JsonDocument.Parse(json);
        Assert.Equal("send_message", doc.RootElement.GetProperty("action").GetString());
        Assert.Equal("sess-1", doc.RootElement.GetProperty("sessionId").GetString());
        Assert.Equal("hello", doc.RootElement.GetProperty("message").GetString());
    }
}
