using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.Postgres.Tests;

/// <summary>
/// The ACL leak contract — run against BOTH the in-memory <c>AclKnowledgeStore</c> and the durable
/// Postgres+pgvector one. The same boundary (anonymous → public-only, entitled → private,
/// unentitled → no leak) must hold whether the ACL lives in memory or is SQL-filtered in Postgres.
/// Mirrors the Rust acl_chat_leak + acl_persistence suites.
/// </summary>
public abstract class AclKnowledgeContractTests
{
    protected abstract Task<IAclKnowledge> CreateAsync();

    private static AccessContext WithGroups(params string[] groups) =>
        new(new Principal("u", "acme", "basic", groups), IsAnonymous: groups.Length == 0);

    private async Task<IAclKnowledge> SeededAsync()
    {
        var store = await CreateAsync();
        await store.IngestAsync(new KnowledgeDocument("pub", "Public support hours are 9 to 5.", "public.md"), DocumentAcl.PublicAcl);
        await store.IngestAsync(
            new KnowledgeDocument("secret", "The private launch code is hunter2.", "acme/private/launch.md"),
            DocumentAcl.ForGroups("github:acme/private"));
        return store;
    }

    [SkippableFact]
    public async Task Anonymous_SeesOnlyPublic()
    {
        var store = await SeededAsync();
        var hits = await store.ForAccess(AccessContext.Anonymous)!.QueryAsync("private launch code", 10);
        Assert.DoesNotContain(hits, h => h.DocumentId == "secret");
    }

    [SkippableFact]
    public async Task EntitledUser_ReadsPrivateDoc()
    {
        var store = await SeededAsync();
        var hits = await store.ForAccess(WithGroups("github:acme/private"))!.QueryAsync("private launch code", 10);
        Assert.Contains(hits, h => h.DocumentId == "secret" && h.Chunk.Contains("hunter2"));
    }

    [SkippableFact]
    public async Task UnentitledUser_NoLeak()
    {
        var store = await SeededAsync();
        var hits = await store.ForAccess(WithGroups("github:acme/other"))!.QueryAsync("private launch code hunter2", 10);
        Assert.DoesNotContain(hits, h => h.DocumentId == "secret");
    }
}

/// <summary>The ACL contract against the in-memory store (always runs).</summary>
public sealed class InMemoryAclKnowledgeContractTests : AclKnowledgeContractTests
{
    protected override Task<IAclKnowledge> CreateAsync() => Task.FromResult<IAclKnowledge>(new AclKnowledgeStore());
}

/// <summary>The ACL contract against the durable Postgres + pgvector store (gated on Docker).</summary>
public sealed class PostgresAclKnowledgeContractTests : AclKnowledgeContractTests, IClassFixture<PostgresFixture>
{
    private readonly PostgresFixture _fixture;

    public PostgresAclKnowledgeContractTests(PostgresFixture fixture) => _fixture = fixture;

    protected override Task<IAclKnowledge> CreateAsync()
    {
        Skip.IfNot(_fixture.Available, "Docker/pgvector unavailable — skipping Postgres ACL knowledge contract.");
        return Task.FromResult<IAclKnowledge>(_fixture.AclKnowledge!);
    }
}
