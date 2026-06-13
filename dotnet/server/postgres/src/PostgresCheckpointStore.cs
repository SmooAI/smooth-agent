using System.Text.Json;
using Microsoft.Extensions.AI;
using Npgsql;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.Postgres;

/// <summary>
/// A durable <see cref="ICheckpointStore"/> on Postgres — agent state survives a process restart,
/// so a long agentic loop can resume instead of restart. Passes the SAME <see cref="ICheckpointStore"/>
/// contract tests as the in-memory store (the adapter-parity pattern), and the C# analog of the Rust
/// engine's <c>PostgresCheckpointStore</c>. Messages are persisted as JSONB (role + text per message);
/// insertion order (a <c>BIGSERIAL seq</c>) is the source of truth for "latest", matching the
/// in-memory store's behavior even when <c>created_at</c> ties within a tick. <c>CREATE TABLE IF NOT
/// EXISTS</c> against the same database the session store uses.
/// </summary>
public sealed class PostgresCheckpointStore : ICheckpointStore, IAsyncDisposable
{
    private const string SchemaSql = """
        CREATE TABLE IF NOT EXISTS agent_checkpoints (
            id          TEXT PRIMARY KEY,
            thread_id   TEXT NOT NULL,
            iteration   INTEGER NOT NULL,
            messages    JSONB NOT NULL,
            metadata    JSONB,
            created_at  TIMESTAMPTZ NOT NULL,
            seq         BIGSERIAL
        );
        CREATE INDEX IF NOT EXISTS idx_checkpoints_thread_seq
            ON agent_checkpoints (thread_id, seq);
        """;

    private static readonly JsonSerializerOptions JsonOptions = new();

    private readonly NpgsqlDataSource _dataSource;

    public PostgresCheckpointStore(string connectionString)
    {
        _dataSource = NpgsqlDataSource.Create(connectionString);
    }

    /// <summary>Create the store and apply the schema (idempotent).</summary>
    public static async Task<PostgresCheckpointStore> CreateAsync(string connectionString, CancellationToken cancellationToken = default)
    {
        var store = new PostgresCheckpointStore(connectionString);
        await store.InitializeAsync(cancellationToken).ConfigureAwait(false);
        return store;
    }

    public async Task InitializeAsync(CancellationToken cancellationToken = default)
    {
        await using var command = _dataSource.CreateCommand(SchemaSql);
        await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    public async Task SaveAsync(Checkpoint checkpoint, CancellationToken cancellationToken = default)
    {
        const string sql = """
            INSERT INTO agent_checkpoints (id, thread_id, iteration, messages, metadata, created_at)
            VALUES (@id, @thread_id, @iteration, @messages::jsonb, @metadata::jsonb, @created_at)
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("id", checkpoint.Id);
        command.Parameters.AddWithValue("thread_id", checkpoint.ThreadId);
        command.Parameters.AddWithValue("iteration", checkpoint.Iteration);
        command.Parameters.AddWithValue("messages", SerializeMessages(checkpoint.Messages));
        command.Parameters.AddWithValue("metadata", (object?)SerializeMetadata(checkpoint.Metadata) ?? DBNull.Value);
        command.Parameters.AddWithValue("created_at", checkpoint.CreatedAt);
        await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    public async Task<Checkpoint?> LoadLatestAsync(string threadId, CancellationToken cancellationToken = default)
    {
        const string sql = """
            SELECT id, thread_id, iteration, messages, metadata, created_at
            FROM agent_checkpoints WHERE thread_id = @thread_id
            ORDER BY seq DESC LIMIT 1
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("thread_id", threadId);
        await using var reader = await command.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);
        return await reader.ReadAsync(cancellationToken).ConfigureAwait(false) ? ReadCheckpoint(reader) : null;
    }

    public async Task<IReadOnlyList<Checkpoint>> ListAsync(string threadId, CancellationToken cancellationToken = default)
    {
        const string sql = """
            SELECT id, thread_id, iteration, messages, metadata, created_at
            FROM agent_checkpoints WHERE thread_id = @thread_id
            ORDER BY seq ASC
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("thread_id", threadId);
        await using var reader = await command.ExecuteReaderAsync(cancellationToken).ConfigureAwait(false);

        var result = new List<Checkpoint>();
        while (await reader.ReadAsync(cancellationToken).ConfigureAwait(false))
        {
            result.Add(ReadCheckpoint(reader));
        }
        return result;
    }

    public async Task<int> PruneAsync(string threadId, int keep, CancellationToken cancellationToken = default)
    {
        // Keep the newest `keep` (by seq); delete the rest. keep <= 0 prunes the whole thread.
        const string sql = """
            DELETE FROM agent_checkpoints
            WHERE thread_id = @thread_id AND seq NOT IN (
                SELECT seq FROM agent_checkpoints WHERE thread_id = @thread_id
                ORDER BY seq DESC LIMIT @keep
            )
            """;
        await using var command = _dataSource.CreateCommand(sql);
        command.Parameters.AddWithValue("thread_id", threadId);
        command.Parameters.AddWithValue("keep", Math.Max(0, keep));
        return await command.ExecuteNonQueryAsync(cancellationToken).ConfigureAwait(false);
    }

    private static Checkpoint ReadCheckpoint(NpgsqlDataReader reader)
    {
        var id = reader.GetString(0);
        var threadId = reader.GetString(1);
        var iteration = reader.GetInt32(2);
        var messages = DeserializeMessages(reader.GetString(3));
        var metadata = reader.IsDBNull(4) ? null : DeserializeMetadata(reader.GetString(4));
        var createdAt = reader.GetFieldValue<DateTimeOffset>(5);
        return new Checkpoint(id, threadId, messages, iteration, createdAt, metadata);
    }

    private static string SerializeMessages(IReadOnlyList<ChatMessage> messages) =>
        JsonSerializer.Serialize(messages.Select(m => new MessageDto(m.Role.Value, m.Text)).ToArray(), JsonOptions);

    private static IReadOnlyList<ChatMessage> DeserializeMessages(string json)
    {
        var dtos = JsonSerializer.Deserialize<MessageDto[]>(json, JsonOptions) ?? Array.Empty<MessageDto>();
        return dtos.Select(d => new ChatMessage(new ChatRole(d.Role), d.Text)).ToArray();
    }

    private static string? SerializeMetadata(IReadOnlyDictionary<string, string>? metadata) =>
        metadata is null ? null : JsonSerializer.Serialize(metadata, JsonOptions);

    private static IReadOnlyDictionary<string, string>? DeserializeMetadata(string json) =>
        JsonSerializer.Deserialize<Dictionary<string, string>>(json, JsonOptions);

    public ValueTask DisposeAsync() => _dataSource.DisposeAsync();

    private sealed record MessageDto(string Role, string Text);
}
