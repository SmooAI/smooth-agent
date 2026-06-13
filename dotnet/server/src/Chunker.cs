namespace SmooAI.SmoothOperator.Server;

/// <summary>How to split a document into chunks for embedding/retrieval.</summary>
public sealed record ChunkingOptions(int MaxChars = 1200, int OverlapChars = 150);

/// <summary>
/// Splits a document into overlapping, size-bounded chunks, preferring to break at whitespace.
/// The C# analog of the Rust engine's chunking pipeline (the G2 gap). Each chunk carries enough
/// overlap that a fact spanning a boundary is still retrievable from one side.
/// </summary>
public static class Chunker
{
    public static IReadOnlyList<string> Chunk(string content, ChunkingOptions options)
    {
        var text = content.Trim();
        if (text.Length == 0)
        {
            return Array.Empty<string>();
        }
        if (text.Length <= options.MaxChars)
        {
            return new[] { text };
        }
        if (options.OverlapChars >= options.MaxChars)
        {
            throw new ArgumentException("OverlapChars must be smaller than MaxChars.", nameof(options));
        }

        var chunks = new List<string>();
        var start = 0;
        while (start < text.Length)
        {
            var end = Math.Min(start + options.MaxChars, text.Length);
            if (end < text.Length)
            {
                // Prefer to break at the last whitespace within this window.
                var window = end - start;
                var whitespace = text.LastIndexOf(' ', end - 1, window);
                if (whitespace > start)
                {
                    end = whitespace;
                }
            }

            var piece = text[start..end].Trim();
            if (piece.Length > 0)
            {
                chunks.Add(piece);
            }

            if (end >= text.Length)
            {
                break;
            }
            start = Math.Max(0, end - options.OverlapChars);
        }

        return chunks;
    }
}
