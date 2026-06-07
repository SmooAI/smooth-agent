// Transport abstraction for the client.
//
// The client is deliberately decoupled from any concrete WebSocket implementation
// so it can be unit-tested with a mock and run against a real socket in production.
// A transport is anything that can send a string frame and surface incoming string
// frames plus lifecycle (close / error) events.

using System.Net.WebSockets;
using System.Text;

namespace SmooAI.SmoothOperatorAgent;

public enum TransportState
{
    Closed,
    Connecting,
    Open,
    Closing,
}

/// <summary>Info surfaced when a transport closes.</summary>
public readonly record struct TransportCloseInfo(int? Code, string? Reason);

/// <summary>
/// Minimal injectable transport contract. Mockable for tests (no live socket needed).
/// </summary>
public interface ITransport
{
    TransportState State { get; }

    /// <summary>Open the connection. Completes once the transport reaches <see cref="TransportState.Open"/>.</summary>
    Task ConnectAsync(CancellationToken cancellationToken = default);

    /// <summary>Send a serialized frame. Throws if the transport is not open.</summary>
    Task SendAsync(string data, CancellationToken cancellationToken = default);

    /// <summary>Close the connection.</summary>
    Task CloseAsync(int code = 1000, string? reason = null, CancellationToken cancellationToken = default);

    /// <summary>Raised for each incoming string frame.</summary>
    event Action<string>? Message;

    /// <summary>Raised when the transport closes.</summary>
    event Action<TransportCloseInfo>? Closed;

    /// <summary>Raised on a transport-level error.</summary>
    event Action<Exception>? Error;
}

/// <summary>
/// Default transport backed by <see cref="ClientWebSocket"/>. Connects to the given
/// URL, pumps incoming text frames on a background receive loop, and raises
/// <see cref="Message"/> / <see cref="Closed"/> / <see cref="Error"/> events.
/// </summary>
public sealed class WebSocketTransport : ITransport, IAsyncDisposable
{
    private readonly Uri _url;
    private readonly Func<ClientWebSocket>? _factory;
    private ClientWebSocket? _socket;
    private CancellationTokenSource? _receiveCts;
    private Task? _receiveLoop;

    public WebSocketTransport(string url, Func<ClientWebSocket>? webSocketFactory = null)
    {
        _url = new Uri(url);
        _factory = webSocketFactory;
    }

    public event Action<string>? Message;
    public event Action<TransportCloseInfo>? Closed;
    public event Action<Exception>? Error;

    public TransportState State => _socket?.State switch
    {
        WebSocketState.Connecting => TransportState.Connecting,
        WebSocketState.Open => TransportState.Open,
        WebSocketState.CloseSent or WebSocketState.CloseReceived => TransportState.Closing,
        _ => TransportState.Closed,
    };

    public async Task ConnectAsync(CancellationToken cancellationToken = default)
    {
        if (_socket is { State: WebSocketState.Open }) return;

        _socket = _factory?.Invoke() ?? new ClientWebSocket();
        await _socket.ConnectAsync(_url, cancellationToken).ConfigureAwait(false);

        _receiveCts = new CancellationTokenSource();
        _receiveLoop = Task.Run(() => ReceiveLoopAsync(_socket, _receiveCts.Token));
    }

    public async Task SendAsync(string data, CancellationToken cancellationToken = default)
    {
        if (_socket is not { State: WebSocketState.Open })
            throw new InvalidOperationException($"Cannot send: transport is \"{State}\".");

        var bytes = Encoding.UTF8.GetBytes(data);
        await _socket.SendAsync(bytes, WebSocketMessageType.Text, endOfMessage: true, cancellationToken)
            .ConfigureAwait(false);
    }

    public async Task CloseAsync(int code = 1000, string? reason = null, CancellationToken cancellationToken = default)
    {
        _receiveCts?.Cancel();
        if (_socket is { State: WebSocketState.Open or WebSocketState.CloseReceived })
        {
            try
            {
                await _socket.CloseAsync((WebSocketCloseStatus)code, reason ?? string.Empty, cancellationToken)
                    .ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                Error?.Invoke(ex);
            }
        }
        Closed?.Invoke(new TransportCloseInfo(code, reason));
    }

    private async Task ReceiveLoopAsync(ClientWebSocket socket, CancellationToken ct)
    {
        var buffer = new byte[8192];
        var message = new MemoryStream();
        try
        {
            while (!ct.IsCancellationRequested && socket.State == WebSocketState.Open)
            {
                WebSocketReceiveResult result;
                message.SetLength(0);
                do
                {
                    result = await socket.ReceiveAsync(buffer, ct).ConfigureAwait(false);
                    if (result.MessageType == WebSocketMessageType.Close)
                    {
                        Closed?.Invoke(new TransportCloseInfo(
                            (int?)result.CloseStatus, result.CloseStatusDescription));
                        return;
                    }
                    message.Write(buffer, 0, result.Count);
                } while (!result.EndOfMessage);

                var text = Encoding.UTF8.GetString(message.GetBuffer(), 0, (int)message.Length);
                Message?.Invoke(text);
            }
        }
        catch (OperationCanceledException)
        {
            // Normal shutdown.
        }
        catch (Exception ex)
        {
            Error?.Invoke(ex);
            Closed?.Invoke(new TransportCloseInfo(null, ex.Message));
        }
    }

    public async ValueTask DisposeAsync()
    {
        _receiveCts?.Cancel();
        if (_receiveLoop is not null)
        {
            try { await _receiveLoop.ConfigureAwait(false); } catch { /* ignore */ }
        }
        _socket?.Dispose();
        _receiveCts?.Dispose();
    }
}
