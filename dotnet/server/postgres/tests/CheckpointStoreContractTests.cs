using Microsoft.Extensions.AI;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.Postgres.Tests;

/// <summary>
/// The <see cref="ICheckpointStore"/> behavioral contract — run against BOTH the in-memory and the
/// Postgres adapter, so agent-state persistence is provably interchangeable. The C# version of the
/// Rust adapter-parity pattern. Each test uses a unique thread id so the shared Postgres store stays
/// isolated across tests.
/// </summary>
public abstract class CheckpointStoreContractTests
{
    /// <summary>Provide a store. May Skip (Postgres when Docker is unavailable).</summary>
    protected abstract Task<ICheckpointStore> CreateStoreAsync();

    private static Checkpoint Cp(string threadId, string id, int iteration, params string[] texts) =>
        new(id, threadId, texts.Select(t => new ChatMessage(ChatRole.User, t)).ToArray(), iteration, DateTimeOffset.UtcNow);

    [SkippableFact]
    public async Task Save_Then_LoadLatest_RoundTrips()
    {
        var store = await CreateStoreAsync();
        var thread = Guid.NewGuid().ToString();
        await store.SaveAsync(Cp(thread, "c1", iteration: 3, "hello", "world"));

        var latest = await store.LoadLatestAsync(thread);

        Assert.NotNull(latest);
        Assert.Equal("c1", latest!.Id);
        Assert.Equal(thread, latest.ThreadId);
        Assert.Equal(3, latest.Iteration);
        Assert.Equal(2, latest.Messages.Count);
        Assert.Equal("hello", latest.Messages[0].Text);
        Assert.Equal(ChatRole.User, latest.Messages[0].Role);
    }

    [SkippableFact]
    public async Task LoadLatest_ReturnsNewestBySaveOrder()
    {
        var store = await CreateStoreAsync();
        var thread = Guid.NewGuid().ToString();
        await store.SaveAsync(Cp(thread, "c1", 1, "first"));
        await store.SaveAsync(Cp(thread, "c2", 2, "second"));

        Assert.Equal("c2", (await store.LoadLatestAsync(thread))!.Id);
    }

    [SkippableFact]
    public async Task List_ReturnsOldestFirst()
    {
        var store = await CreateStoreAsync();
        var thread = Guid.NewGuid().ToString();
        await store.SaveAsync(Cp(thread, "c1", 1, "a"));
        await store.SaveAsync(Cp(thread, "c2", 2, "b"));
        await store.SaveAsync(Cp(thread, "c3", 3, "c"));

        var all = await store.ListAsync(thread);

        Assert.Equal(new[] { "c1", "c2", "c3" }, all.Select(c => c.Id));
    }

    [SkippableFact]
    public async Task Prune_KeepsNewestN()
    {
        var store = await CreateStoreAsync();
        var thread = Guid.NewGuid().ToString();
        for (var i = 1; i <= 5; i++)
        {
            await store.SaveAsync(Cp(thread, $"c{i}", i, "x"));
        }

        var removed = await store.PruneAsync(thread, keep: 2);

        Assert.Equal(3, removed);
        Assert.Equal(new[] { "c4", "c5" }, (await store.ListAsync(thread)).Select(c => c.Id));
    }

    [SkippableFact]
    public async Task LoadLatest_UnknownThread_ReturnsNull()
    {
        var store = await CreateStoreAsync();
        Assert.Null(await store.LoadLatestAsync(Guid.NewGuid().ToString()));
    }

    [SkippableFact]
    public async Task Threads_AreIsolated()
    {
        var store = await CreateStoreAsync();
        var a = Guid.NewGuid().ToString();
        var b = Guid.NewGuid().ToString();
        await store.SaveAsync(Cp(a, "a1", 1, "a-state"));
        await store.SaveAsync(Cp(b, "b1", 1, "b-state"));

        Assert.Equal("a1", (await store.LoadLatestAsync(a))!.Id);
        Assert.Single(await store.ListAsync(a));
    }
}

public sealed class InMemoryCheckpointStoreContractTests : CheckpointStoreContractTests
{
    protected override Task<ICheckpointStore> CreateStoreAsync() =>
        Task.FromResult<ICheckpointStore>(new InMemoryCheckpointStore());
}

public sealed class PostgresCheckpointStoreContractTests : CheckpointStoreContractTests, IClassFixture<PostgresFixture>
{
    private readonly PostgresFixture _fixture;

    public PostgresCheckpointStoreContractTests(PostgresFixture fixture) => _fixture = fixture;

    protected override Task<ICheckpointStore> CreateStoreAsync()
    {
        Skip.IfNot(_fixture.Available, "Docker/Postgres unavailable — skipping Postgres checkpoint contract.");
        return Task.FromResult<ICheckpointStore>(_fixture.CheckpointStore!);
    }
}
