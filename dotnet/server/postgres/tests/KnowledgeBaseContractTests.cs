using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.Postgres.Tests;

/// <summary>
/// The <see cref="IKnowledgeBase"/> behavioral contract — run against BOTH the engine's in-memory
/// (lexical) store and the Postgres+pgvector (vector) store. Different internals, same contract:
/// ingest then retrieve the relevant document; ingest is idempotent by id.
/// </summary>
public abstract class KnowledgeBaseContractTests
{
    protected abstract Task<IKnowledgeBase> CreateAsync();

    [SkippableFact]
    public async Task Ingest_Then_Query_RanksRelevantDocFirst()
    {
        var kb = await CreateAsync();
        await kb.IngestAsync(new KnowledgeDocument("returns", "Our return window is 17 days from delivery.", "returns.md"));
        await kb.IngestAsync(new KnowledgeDocument("shipping", "Standard shipping takes 5 to 7 business days.", "shipping.md"));

        var hits = await kb.QueryAsync("how long is the return window", 4);

        Assert.NotEmpty(hits);
        Assert.Equal("returns", hits[0].DocumentId);
        Assert.Contains("17 days", hits[0].Chunk);
    }

    [SkippableFact]
    public async Task Ingest_IsIdempotentById()
    {
        var kb = await CreateAsync();
        await kb.IngestAsync(new KnowledgeDocument("doc-x", "original placeholder text", "x.md"));
        await kb.IngestAsync(new KnowledgeDocument("doc-x", "the refreshed payload mentions wombats", "x.md"));

        var hits = await kb.QueryAsync("refreshed payload wombats", 4);

        Assert.Contains(hits, h => h.DocumentId == "doc-x" && h.Chunk.Contains("wombats"));
        // Not duplicated — a single row per id.
        Assert.Single(hits.Where(h => h.DocumentId == "doc-x"));
    }
}

/// <summary>The contract, against the engine's in-memory knowledge base (always runs).</summary>
public sealed class InMemoryKnowledgeBaseContractTests : KnowledgeBaseContractTests
{
    protected override Task<IKnowledgeBase> CreateAsync() => Task.FromResult<IKnowledgeBase>(new InMemoryKnowledgeBase());
}

/// <summary>The contract, against Postgres + pgvector (gated on Docker).</summary>
public sealed class PostgresKnowledgeBaseContractTests : KnowledgeBaseContractTests, IClassFixture<PostgresFixture>
{
    private readonly PostgresFixture _fixture;

    public PostgresKnowledgeBaseContractTests(PostgresFixture fixture) => _fixture = fixture;

    protected override Task<IKnowledgeBase> CreateAsync()
    {
        Skip.IfNot(_fixture.Available, "Docker/pgvector unavailable — skipping Postgres knowledge contract.");
        return Task.FromResult<IKnowledgeBase>(_fixture.Knowledge!);
    }
}
