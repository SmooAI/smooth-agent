using SmooAI.SmoothOperator.Server;
using Testcontainers.PostgreSql;

namespace SmooAI.SmoothOperator.Server.Postgres.Tests;

/// <summary>
/// Spins up a real Postgres in a container for the class. If Docker is unavailable, it degrades
/// to "unavailable" and the tests skip cleanly (never fail) — matching the repo's gated-test rule.
/// </summary>
public sealed class PostgresFixture : IAsyncLifetime
{
    private PostgreSqlContainer? _container;

    public PostgresSessionStore? Store { get; private set; }

    public PostgresKnowledgeBase? Knowledge { get; private set; }

    public PostgresAclKnowledgeStore? AclKnowledge { get; private set; }

    public PostgresCheckpointStore? CheckpointStore { get; private set; }

    public string? ConnectionString { get; private set; }

    public bool Available => Store is not null && ConnectionString is not null;

    public async Task InitializeAsync()
    {
        try
        {
            // The pgvector image is a superset of postgres — serves both the OLTP session store
            // and the vector-searched knowledge adapters from one container.
            _container = new PostgreSqlBuilder().WithImage("pgvector/pgvector:pg16").Build();
            await _container.StartAsync();
            ConnectionString = _container.GetConnectionString();
            Store = await PostgresSessionStore.CreateAsync(ConnectionString);
            Knowledge = await PostgresKnowledgeBase.CreateAsync(ConnectionString, new DeterministicEmbedder(256));
            AclKnowledge = await PostgresAclKnowledgeStore.CreateAsync(ConnectionString, new DeterministicEmbedder(256));
            CheckpointStore = await PostgresCheckpointStore.CreateAsync(ConnectionString);
        }
        catch
        {
            // Docker not reachable — leave Available == false so tests skip.
            Store = null;
            Knowledge = null;
            AclKnowledge = null;
            CheckpointStore = null;
            ConnectionString = null;
        }
    }

    public async Task DisposeAsync()
    {
        if (CheckpointStore is not null)
        {
            await CheckpointStore.DisposeAsync();
        }
        if (AclKnowledge is not null)
        {
            await AclKnowledge.DisposeAsync();
        }
        if (Knowledge is not null)
        {
            await Knowledge.DisposeAsync();
        }
        if (Store is not null)
        {
            await Store.DisposeAsync();
        }
        if (_container is not null)
        {
            await _container.DisposeAsync();
        }
    }
}

/// <summary>The shared contract, against the Postgres adapter (gated on Docker).</summary>
public sealed class PostgresSessionStoreContractTests : SessionStoreContractTests, IClassFixture<PostgresFixture>
{
    private readonly PostgresFixture _fixture;

    public PostgresSessionStoreContractTests(PostgresFixture fixture) => _fixture = fixture;

    protected override Task<ISessionStore> CreateStoreAsync()
    {
        Skip.IfNot(_fixture.Available, "Docker/Postgres unavailable — skipping Postgres adapter contract.");
        return Task.FromResult<ISessionStore>(_fixture.Store!);
    }

    [SkippableFact]
    public async Task Session_And_History_SurviveAcrossStoreInstances()
    {
        Skip.IfNot(_fixture.Available, "Docker/Postgres unavailable — skipping durability test.");

        // "Process 1" writes…
        await using var first = await PostgresSessionStore.CreateAsync(_fixture.ConnectionString!);
        var session = await first.CreateSessionAsync("", "Bob", null);
        await first.AppendMessageAsync(session.ConversationId, MessageDirection.Inbound, "persist me");

        // …a fresh store instance ("restart") still sees the durable session + history.
        await using var second = await PostgresSessionStore.CreateAsync(_fixture.ConnectionString!);
        var fetched = await second.GetSessionAsync(session.SessionId);
        Assert.NotNull(fetched);
        Assert.Equal(session.ConversationId, fetched!.ConversationId);

        var messages = await second.ListMessagesAsync(session.ConversationId, 50);
        Assert.Single(messages);
        Assert.Equal("persist me", messages[0].Text);
    }
}
