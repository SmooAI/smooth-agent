using System.Text.Json;

namespace SmooAI.SmoothOperatorAgent.Tests;

/// <summary>In-memory transport: captures sent frames, lets the test inject server events.</summary>
internal sealed class MockTransport : ITransport
{
    public TransportState State { get; private set; } = TransportState.Closed;
    public List<string> Sent { get; } = new();

    public event Action<string>? Message;
    public event Action<TransportCloseInfo>? Closed;
    public event Action<Exception>? Error;

    public Task ConnectAsync(CancellationToken cancellationToken = default)
    {
        State = TransportState.Open;
        return Task.CompletedTask;
    }

    public Task SendAsync(string data, CancellationToken cancellationToken = default)
    {
        if (State != TransportState.Open)
            throw new InvalidOperationException($"not open: {State}");
        Sent.Add(data);
        return Task.CompletedTask;
    }

    public Task CloseAsync(int code = 1000, string? reason = null, CancellationToken cancellationToken = default)
    {
        State = TransportState.Closed;
        Closed?.Invoke(new TransportCloseInfo(code, reason));
        return Task.CompletedTask;
    }

    /// <summary>Simulate a server→client event from a raw JSON string.</summary>
    public void Emit(string json) => Message?.Invoke(json);

    /// <summary>The last action frame the client sent, parsed.</summary>
    public JsonElement LastSent()
        => JsonDocument.Parse(Sent[^1]).RootElement.Clone();

    public string LastRequestId() => LastSent().GetProperty("requestId").GetString()!;

    public void RaiseError(Exception ex) => Error?.Invoke(ex);
}
