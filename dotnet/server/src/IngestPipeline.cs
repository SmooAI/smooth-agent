using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server;

/// <summary>What an ingest run produced.</summary>
public sealed record IngestResult(int Documents, int Chunks);

/// <summary>
/// Pulls documents from an <see cref="IConnector"/>, chunks them, and ingests each chunk into an
/// <see cref="IKnowledgeBase"/> (which embeds + stores). The C# analog of the Rust ingest pipeline:
/// connector → chunk → embed → store. After a run, the source's content is retrievable.
/// </summary>
public sealed class IngestPipeline
{
    private readonly IKnowledgeBase _knowledge;
    private readonly ChunkingOptions _chunking;

    public IngestPipeline(IKnowledgeBase knowledge, ChunkingOptions? chunking = null)
    {
        _knowledge = knowledge ?? throw new ArgumentNullException(nameof(knowledge));
        _chunking = chunking ?? new ChunkingOptions();
    }

    public async Task<IngestResult> IngestAsync(IConnector connector, CancellationToken cancellationToken = default)
    {
        var documents = 0;
        var chunks = 0;

        await foreach (var document in connector.PullAsync(cancellationToken).ConfigureAwait(false))
        {
            documents++;
            var pieces = Chunker.Chunk(document.Content, _chunking);
            for (var i = 0; i < pieces.Count; i++)
            {
                // A single chunk keeps the document's own id; multiple chunks get a stable suffix.
                var chunkId = pieces.Count == 1 ? document.Id : $"{document.Id}#chunk-{i}";
                await _knowledge.IngestAsync(
                    new KnowledgeDocument(chunkId, pieces[i], document.Source, document.DocType),
                    cancellationToken).ConfigureAwait(false);
                chunks++;
            }
        }

        return new IngestResult(documents, chunks);
    }
}
