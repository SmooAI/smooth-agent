using Npgsql;
using Pgvector;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.Postgres;

/// <summary>
/// A durable, vector-searched <see cref="IKnowledgeBase"/> backed by Postgres + pgvector — the C#
/// analog of the Rust Postgres knowledge store. Documents are embedded (via an injected
/// <see cref="IEmbedder"/>) and stored as <c>vector</c> rows; <see cref="QueryAsync"/> embeds the
/// query and ranks by cosine distance (<c>&lt;=&gt;</c>). Satisfies the same <see cref="IKnowledgeBase"/>
/// contract as the engine's in-memory store.
/// </summary>
public sealed class PostgresKnowledgeBase : IKnowledgeBase, IAsyncDisposable
{
    private readonly NpgsqlDataSource _dataSource;
    private readonly IEmbedder _embedder;

    private PostgresKnowledgeBase(NpgsqlDataSource dataSource, IEmbedder embedder)
    {
        _dataSource = dataSource;
        _embedder = embedder;
    }

    public static async Task<PostgresKnowledgeBase> CreateAsync(string connectionString, IEmbedder embedder, CancellationToken cancellationToken = default)
    {
        // 1. Ensure the extension exists FIRST (a plain connection — the vector type isn't mapped yet).
        await using (var connection = new NpgsqlConnection(connectionString))
        {
            await connection.OpenAsync(cancellationToken).ConfigureAwait(false);
            await using var extension = new NpgsqlCommand("CREATE EXTENSION IF NOT EXISTS vector;", connection);
            await extension.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }

        // 2. Now build a data source with the vector type mapped (the extension exists).
        var builder = new NpgsqlDataSourceBuilder(connectionString);
        builder.UseVector();
        var dataSource = builder.Build();

        // 3. Create the table with a vector column sized to the embedder.
        var schema =
            $"""
            CREATE TABLE IF NOT EXISTS knowledge_documents (
                id        TEXT PRIMARY KEY,
                content   TEXT NOT NULL,
                source    TEXT NOT NULL,
                embedding vector({embedder.Dimensions})
            );
            """;
        await using (var command = dataSource.CreateCommand(schema))
        {
            await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
        }

        return new PostgresKnowledgeBase(dataSource, embedder);
    }

    public async Task IngestAsync(KnowledgeDocument document, CancellationToken cancellationToken = default)
    {
        var embedding = await _embedder.EmbedAsync(document.Content, cancellationToken).ConfigureAwait(false);

        const string sql = """
            INSERT INTO knowledge_documents (id, content, source, embedding)
            VALUES (@id, @content, @source, @embedding)
            ON CONFLICT (id) DO UPDATE SET content = @content, source = @source, embedding = @embedding
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("id", document.Id);
        command.Parameters.AddWithValue("content", document.Content);
        command.Parameters.AddWithValue("source", document.Source);
        command.Parameters.AddWithValue("embedding", new Vector(embedding));
        await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<KnowledgeResult>> QueryAsync(string query, int limit, CancellationToken cancellationToken = default)
    {
        var embedding = await _embedder.EmbedAsync(query, cancellationToken).ConfigureAwait(false);

        // `<=>` is cosine distance; 1 - distance is cosine similarity (the score).
        const string sql = """
            SELECT id, content, source, 1 - (embedding <=> @q) AS score
            FROM knowledge_documents
            WHERE embedding IS NOT NULL
            ORDER BY embedding <=> @q
            LIMIT @lim
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("q", new Vector(embedding));
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
}
