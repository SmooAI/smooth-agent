using Npgsql;
using Pgvector;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.Postgres;

/// <summary>
/// A durable, ACL-aware, vector-searched knowledge store backed by Postgres + pgvector. Each
/// document carries an ACL (public, or restricted to entitlement groups) persisted in
/// <c>acl_public</c> / <c>acl_groups</c>, and retrieval filters by the caller's groups IN SQL
/// before ranking — the C# analog of the Rust <c>knowledge_vectors.acl</c> SQL filter that makes
/// the leak boundary survive the ingest→serve process boundary. Satisfies the same
/// <see cref="IAclKnowledge"/> contract as the in-memory <see cref="AclKnowledgeStore"/>.
/// </summary>
public sealed class PostgresAclKnowledgeStore : IAclKnowledge, IAsyncDisposable
{
    private readonly NpgsqlDataSource _dataSource;
    private readonly IEmbedder _embedder;

    private PostgresAclKnowledgeStore(NpgsqlDataSource dataSource, IEmbedder embedder)
    {
        _dataSource = dataSource;
        _embedder = embedder;
    }

    public static async Task<PostgresAclKnowledgeStore> CreateAsync(string connectionString, IEmbedder embedder, CancellationToken cancellationToken = default)
    {
        await using (var connection = new NpgsqlConnection(connectionString))
        {
            await connection.OpenAsync(cancellationToken).ConfigureAwait(false);
            await using var extension = new NpgsqlCommand("CREATE EXTENSION IF NOT EXISTS vector;", connection);
            await extension.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }

        var builder = new NpgsqlDataSourceBuilder(connectionString);
        builder.UseVector();
        var dataSource = builder.Build();

        var schema =
            $"""
            CREATE TABLE IF NOT EXISTS acl_knowledge_documents (
                id         TEXT PRIMARY KEY,
                content    TEXT NOT NULL,
                source     TEXT NOT NULL,
                embedding  vector({embedder.Dimensions}),
                acl_public BOOLEAN NOT NULL DEFAULT true,
                acl_groups TEXT[]  NOT NULL DEFAULT ARRAY[]::TEXT[]
            );
            """;
        await using (var command = dataSource.CreateCommand(schema))
        {
            await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }

        return new PostgresAclKnowledgeStore(dataSource, embedder);
    }

    public async Task IngestAsync(KnowledgeDocument document, DocumentAcl acl, CancellationToken cancellationToken = default)
    {
        var embedding = await _embedder.EmbedAsync(document.Content, cancellationToken).ConfigureAwait(false);

        const string sql = """
            INSERT INTO acl_knowledge_documents (id, content, source, embedding, acl_public, acl_groups)
            VALUES (@id, @content, @source, @embedding, @public, @groups)
            ON CONFLICT (id) DO UPDATE SET
                content = @content, source = @source, embedding = @embedding,
                acl_public = @public, acl_groups = @groups
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("id", document.Id);
        command.Parameters.AddWithValue("content", document.Content);
        command.Parameters.AddWithValue("source", document.Source);
        command.Parameters.AddWithValue("embedding", new Vector(embedding));
        command.Parameters.AddWithValue("public", acl.Public);
        command.Parameters.AddWithValue("groups", acl.Groups.ToArray());
        await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    public IKnowledgeBase ForAccess(AccessContext access) => new ScopedView(this, access);

    public IKnowledgeBase WithAcl(DocumentAcl acl) => new IngestView(this, acl);

    private async Task<IReadOnlyList<KnowledgeResult>> QueryForAccessAsync(string query, int limit, AccessContext access, CancellationToken cancellationToken)
    {
        var embedding = await _embedder.EmbedAsync(query, cancellationToken).ConfigureAwait(false);

        // ACL filter IN SQL: a doc is visible if it's public, or its groups overlap the caller's
        // (`&&` is array-overlap). Anonymous (no groups) gets public only — fail-closed.
        const string sql = """
            SELECT id, content, source, 1 - (embedding <=> @q) AS score
            FROM acl_knowledge_documents
            WHERE embedding IS NOT NULL AND (acl_public OR acl_groups && @groups)
            ORDER BY embedding <=> @q
            LIMIT @lim
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("q", new Vector(embedding));
        command.Parameters.AddWithValue("groups", access.Groups.ToArray());
        command.Parameters.AddWithValue("lim", limit);

        var results = new List<KnowledgeResult>();
        await using var reader = await command.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            results.Add(new KnowledgeResult(
                DocumentId: reader.GetString(0),
                Chunk: reader.GetString(1),
                Score: reader.GetDouble(3),
                Source: reader.GetString(2)));
        }
        return results;
    }

    public ValueTask DisposeAsync() => _dataSource.DisposeAsync();

    private sealed class ScopedView : IKnowledgeBase
    {
        private readonly PostgresAclKnowledgeStore _store;
        private readonly AccessContext _access;

        public ScopedView(PostgresAclKnowledgeStore store, AccessContext access)
        {
            _store = store;
            _access = access;
        }

        public Task IngestAsync(KnowledgeDocument document, CancellationToken cancellationToken = default) =>
            throw new NotSupportedException("An access-scoped knowledge view is read-only; ingest through the store.");

        public Task<IReadOnlyList<KnowledgeResult>> QueryAsync(string query, int limit, CancellationToken cancellationToken = default) =>
            _store.QueryForAccessAsync(query, limit, _access, cancellationToken);
    }

    private sealed class IngestView : IKnowledgeBase
    {
        private readonly PostgresAclKnowledgeStore _store;
        private readonly DocumentAcl _acl;

        public IngestView(PostgresAclKnowledgeStore store, DocumentAcl acl)
        {
            _store = store;
            _acl = acl;
        }

        public Task IngestAsync(KnowledgeDocument document, CancellationToken cancellationToken = default) =>
            _store.IngestAsync(document, _acl, cancellationToken);

        public Task<IReadOnlyList<KnowledgeResult>> QueryAsync(string query, int limit, CancellationToken cancellationToken = default) =>
            throw new NotSupportedException("An ingest view is write-only; query through ForAccess(access).");
    }
}
