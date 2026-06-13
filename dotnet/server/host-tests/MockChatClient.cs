using System.Runtime.CompilerServices;
using Microsoft.Extensions.AI;

namespace SmooAI.SmoothOperator.Server.Host.Tests;

internal sealed class MockChatClient : IChatClient
{
    private readonly Queue<ChatResponse> _responses = new();

    public MockChatClient PushText(string text)
    {
        _responses.Enqueue(new ChatResponse(new ChatMessage(ChatRole.Assistant, text)) { ModelId = "mock-model" });
        return this;
    }

    private ChatResponse Next() =>
        _responses.Count > 0 ? _responses.Dequeue() : new ChatResponse(new ChatMessage(ChatRole.Assistant, string.Empty));

    public Task<ChatResponse> GetResponseAsync(IEnumerable<ChatMessage> messages, ChatOptions? options = null, CancellationToken cancellationToken = default) =>
        Task.FromResult(Next());

    public async IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        foreach (var update in Next().ToChatResponseUpdates())
        {
            await Task.Yield();
            yield return update;
        }
    }

    public object? GetService(Type serviceType, object? serviceKey = null) => null;

    public void Dispose()
    {
    }
}
