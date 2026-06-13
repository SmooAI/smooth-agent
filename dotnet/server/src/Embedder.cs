namespace SmooAI.SmoothOperator.Server;

/// <summary>
/// Turns text into an embedding vector for similarity search. Mirrors the Rust engine's embedder
/// seam: a real gateway embedder for production, a deterministic one for tests / offline use.
/// </summary>
public interface IEmbedder
{
    int Dimensions { get; }

    Task<float[]> EmbedAsync(string text, CancellationToken cancellationToken = default);
}

/// <summary>
/// A deterministic, network-free embedder — hashed bag-of-words into a fixed-dimension, L2-normalized
/// vector. Same text → same vector, and texts sharing tokens are close in cosine space. The C# analog
/// of the Rust <c>DeterministicEmbedder</c>; ideal for tests + small in-process corpora.
/// </summary>
public sealed class DeterministicEmbedder : IEmbedder
{
    public int Dimensions { get; }

    public DeterministicEmbedder(int dimensions = 256)
    {
        if (dimensions <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(dimensions));
        }
        Dimensions = dimensions;
    }

    public Task<float[]> EmbedAsync(string text, CancellationToken cancellationToken = default)
    {
        var vector = new float[Dimensions];
        foreach (var token in Tokenize(text))
        {
            var slot = (int)(Fnv1a(token) % (uint)Dimensions);
            vector[slot] += 1f;
        }

        // L2-normalize so cosine distance is well-behaved.
        double sumOfSquares = 0;
        foreach (var value in vector)
        {
            sumOfSquares += value * value;
        }
        if (sumOfSquares > 0)
        {
            var norm = (float)Math.Sqrt(sumOfSquares);
            for (var i = 0; i < vector.Length; i++)
            {
                vector[i] /= norm;
            }
        }

        return Task.FromResult(vector);
    }

    private static IEnumerable<string> Tokenize(string text)
    {
        foreach (var raw in text.ToLowerInvariant().Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries))
        {
            var token = new string(raw.Where(char.IsLetterOrDigit).ToArray());
            if (token.Length > 2)
            {
                yield return token;
            }
        }
    }

    private static uint Fnv1a(string value)
    {
        uint hash = 2166136261;
        foreach (var c in value)
        {
            hash ^= c;
            hash *= 16777619;
        }
        return hash;
    }
}
