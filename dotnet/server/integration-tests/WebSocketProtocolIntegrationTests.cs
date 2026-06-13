using System.Net.WebSockets;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using SmooAI.SmoothOperator.Server.AspNetCore;

namespace SmooAI.SmoothOperator.Server.IntegrationTests;

/// <summary>
/// End-to-end integration tests: boot the ASP.NET Core WebSocket host in-process and drive the
/// wire protocol over a REAL WebSocket — the C# parity of the Rust server's
/// <c>tests/protocol_smoke.rs</c>. CI-safe (a scripted mock IChatClient, no gateway).
/// </summary>
public class WebSocketProtocolIntegrationTests
{
    private static WebApplication BuildApp(IChatClient chat)
    {
        var builder = WebApplication.CreateBuilder();
        builder.WebHost.UseTestServer();
        builder.Services.AddSingleton(chat);
        builder.Services.AddSmoothOperatorServer();

        var app = builder.Build();
        app.MapSmoothOperatorWebSocket("/ws");
        return app;
    }

    private static Task SendAsync(WebSocket socket, string json) =>
        socket.SendAsync(Encoding.UTF8.GetBytes(json), WebSocketMessageType.Text, endOfMessage: true, CancellationToken.None);

    private static async Task<JsonObject> ReceiveAsync(WebSocket socket)
    {
        var buffer = new byte[16 * 1024];
        using var stream = new MemoryStream();
        WebSocketReceiveResult result;
        do
        {
            result = await socket.ReceiveAsync(buffer, CancellationToken.None);
            stream.Write(buffer, 0, result.Count);
        }
        while (!result.EndOfMessage);
        return JsonNode.Parse(Encoding.UTF8.GetString(stream.ToArray()))!.AsObject();
    }

    private static async Task<WebSocket> ConnectAsync(TestServer server)
    {
        var client = server.CreateWebSocketClient();
        return await client.ConnectAsync(new Uri(server.BaseAddress, "ws"), CancellationToken.None);
    }

    [Fact]
    public async Task FullConversation_OverRealWebSocket()
    {
        await using var app = BuildApp(new MockChatClient().PushText("Your return window is 17 days."));
        await app.StartAsync();
        using var socket = await ConnectAsync(app.GetTestServer());

        // 1. ping → pong (mirrors protocol_smoke ping_returns_pong)
        await SendAsync(socket, """{"action":"ping","requestId":"ping-1"}""");
        var pong = await ReceiveAsync(socket);
        Assert.Equal("pong", pong["type"]!.GetValue<string>());
        Assert.Equal("ping-1", pong["requestId"]!.GetValue<string>());

        // 2. create_conversation_session → descriptor (mirrors create_session_returns_valid_descriptor)
        var agentId = Guid.NewGuid().ToString();
        await SendAsync(socket, $$"""{"action":"create_conversation_session","requestId":"cs-1","agentId":"{{agentId}}","userName":"Test"}""");
        var created = await ReceiveAsync(socket);
        Assert.Equal("immediate_response", created["type"]!.GetValue<string>());
        Assert.Equal(200, created["status"]!.GetValue<int>());
        var sessionId = created["data"]!["sessionId"]!.GetValue<string>();
        Assert.True(Guid.TryParse(sessionId, out _), "sessionId must be a UUID");
        Assert.True(Guid.TryParse(created["data"]!["conversationId"]!.GetValue<string>(), out _));
        Assert.Equal(agentId, created["data"]!["agentId"]!.GetValue<string>()); // echoed back

        // 3. send_message → 202 ack → stream_token(s) → eventual_response (the happy path the mock enables)
        await SendAsync(socket, $$"""{"action":"send_message","requestId":"sm-1","sessionId":"{{sessionId}}","message":"How long can I return?"}""");
        var ack = await ReceiveAsync(socket);
        Assert.Equal("immediate_response", ack["type"]!.GetValue<string>());
        Assert.Equal(202, ack["status"]!.GetValue<int>());

        var sawToken = false;
        JsonObject ev;
        do
        {
            ev = await ReceiveAsync(socket);
            if (ev["type"]!.GetValue<string>() == "stream_token")
            {
                sawToken = true;
            }
        }
        while (ev["type"]!.GetValue<string>() != "eventual_response");

        Assert.True(sawToken, "expected at least one stream_token before the terminal event");
        var parts = ev["data"]!["data"]!["response"]!["responseParts"]!.AsArray();
        Assert.Contains(parts, p => p!.GetValue<string>().Contains("17 days"));

        await socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "done", CancellationToken.None);
        await app.StopAsync();
    }

    [Fact]
    public async Task UnknownAction_ErrorsWithoutDroppingConnection()
    {
        await using var app = BuildApp(new MockChatClient());
        await app.StartAsync();
        using var socket = await ConnectAsync(app.GetTestServer());

        await SendAsync(socket, """{"action":"frobnicate","requestId":"x1"}""");
        var error = await ReceiveAsync(socket);
        Assert.Equal("error", error["type"]!.GetValue<string>());

        // The connection survives — a subsequent ping still works (mirrors
        // unknown_action_errors_without_dropping_connection).
        await SendAsync(socket, """{"action":"ping","requestId":"ping-2"}""");
        var pong = await ReceiveAsync(socket);
        Assert.Equal("pong", pong["type"]!.GetValue<string>());
        Assert.Equal("ping-2", pong["requestId"]!.GetValue<string>());

        await socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "done", CancellationToken.None);
        await app.StopAsync();
    }
}
