using System.Net.WebSockets;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;

namespace SmooAI.SmoothOperator.Server.Host.Tests;

/// <summary>
/// Boots the deployable host in-process (WebApplicationFactory) with a scripted model, and proves
/// it actually serves: /health responds, and the protocol works over a real WebSocket. The model
/// is overridden so no gateway key is needed — CI-safe.
/// </summary>
public class HostBootTests : IClassFixture<WebApplicationFactory<Program>>
{
    private readonly WebApplicationFactory<Program> _factory;

    public HostBootTests(WebApplicationFactory<Program> factory) =>
        _factory = factory.WithWebHostBuilder(builder =>
            builder.ConfigureTestServices(services => services.AddSingleton<IChatClient>(new MockChatClient().PushText("ok"))));

    [Fact]
    public async Task Health_ReturnsOk()
    {
        var client = _factory.CreateClient();
        var response = await client.GetAsync("/health");
        response.EnsureSuccessStatusCode();
        Assert.Contains("\"status\":\"ok\"", await response.Content.ReadAsStringAsync());
    }

    [Fact]
    public async Task WebSocket_Ping_Pong()
    {
        _ = _factory.CreateClient(); // ensure the server is started
        using var socket = await _factory.Server.CreateWebSocketClient()
            .ConnectAsync(new Uri(_factory.Server.BaseAddress, "ws"), CancellationToken.None);

        await socket.SendAsync(Encoding.UTF8.GetBytes("""{"action":"ping","requestId":"p1"}"""), WebSocketMessageType.Text, true, CancellationToken.None);

        var buffer = new byte[4096];
        var result = await socket.ReceiveAsync(buffer, CancellationToken.None);
        var ev = JsonNode.Parse(Encoding.UTF8.GetString(buffer, 0, result.Count))!.AsObject();

        Assert.Equal("pong", ev["type"]!.GetValue<string>());
        Assert.Equal("p1", ev["requestId"]!.GetValue<string>());

        await socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "done", CancellationToken.None);
    }
}
