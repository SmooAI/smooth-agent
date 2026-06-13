using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Serialization;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server;

/// <summary>
/// Pulls text/code documents from a GitHub repository — the C# analog of the Rust GitHub connector.
/// Lists the repo tree (git trees API, recursive), then fetches each text file's raw content. The
/// caller supplies a configured <see cref="HttpClient"/> (User-Agent + Authorization for private
/// repos / higher rate limits). Network parsing is unit-tested against a fake handler, so the
/// connector logic runs in CI without hitting GitHub.
/// </summary>
public sealed class GitHubConnector : IConnector
{
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    private static readonly HashSet<string> TextExtensions = new(StringComparer.OrdinalIgnoreCase)
    {
        ".md", ".markdown", ".mdx", ".txt", ".rst", ".adoc",
        ".cs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java", ".rb", ".php", ".cpp", ".cc", ".c", ".h", ".hpp",
        ".json", ".yaml", ".yml", ".toml", ".sql", ".sh",
    };

    private static readonly string[] CodeExtensions =
    {
        ".cs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java", ".rb", ".php", ".cpp", ".cc", ".c", ".h", ".hpp", ".sql", ".sh",
    };

    private readonly HttpClient _http;
    private readonly string _owner;
    private readonly string _repo;
    private readonly string _ref;

    public GitHubConnector(string owner, string repo, HttpClient httpClient, string reference = "main")
    {
        _owner = owner;
        _repo = repo;
        _http = httpClient ?? throw new ArgumentNullException(nameof(httpClient));
        _ref = reference;
    }

    public async IAsyncEnumerable<SourceDocument> PullAsync([EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        TreeResponse? tree;
        var treeUrl = $"https://api.github.com/repos/{_owner}/{_repo}/git/trees/{_ref}?recursive=1";
        using (var response = await _http.GetAsync(treeUrl, cancellationToken).ConfigureAwait(false))
        {
            response.EnsureSuccessStatusCode();
            await using var stream = await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
            tree = await JsonSerializer.DeserializeAsync<TreeResponse>(stream, JsonOptions, cancellationToken).ConfigureAwait(false);
        }

        if (tree?.Tree is null)
        {
            yield break;
        }

        // GitHub's recursive trees API returns a PARTIAL tree with truncated=true when the repo is
        // too large (>100k entries / >7MB). Ingesting that partial tree silently would index an
        // incomplete repo and report success — answers would be confidently incomplete. Fail loud so
        // the operator sees it (surfaced as a per-repo error) and can ingest sub-paths instead.
        if (tree.Truncated)
        {
            throw new InvalidOperationException(
                $"GitHub tree for {_owner}/{_repo}@{_ref} is truncated (repo too large for the recursive trees API); " +
                "ingestion would be incomplete. Ingest narrower paths/refs instead.");
        }

        foreach (var entry in tree.Tree)
        {
            if (entry.Type != "blob" || string.IsNullOrEmpty(entry.Path) || !IsTextFile(entry.Path))
            {
                continue;
            }

            var rawUrl = $"https://raw.githubusercontent.com/{_owner}/{_repo}/{_ref}/{entry.Path}";
            string content;
            using (var response = await _http.GetAsync(rawUrl, cancellationToken).ConfigureAwait(false))
            {
                if (!response.IsSuccessStatusCode)
                {
                    continue;
                }
                content = await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
            }

            yield return new SourceDocument(
                Id: $"{_owner}/{_repo}@{_ref}#{entry.Path}",
                Source: $"{_owner}/{_repo}/{entry.Path}",
                Content: content,
                DocType: DocTypeFor(entry.Path));
        }
    }

    private static bool IsTextFile(string path)
    {
        var dot = path.LastIndexOf('.');
        return dot >= 0 && TextExtensions.Contains(path[dot..]);
    }

    private static DocumentType DocTypeFor(string path)
    {
        var dot = path.LastIndexOf('.');
        if (dot < 0)
        {
            return DocumentType.Documentation;
        }
        var ext = path[dot..];
        if (ext.Equals(".md", StringComparison.OrdinalIgnoreCase) || ext.Equals(".markdown", StringComparison.OrdinalIgnoreCase) || ext.Equals(".mdx", StringComparison.OrdinalIgnoreCase))
        {
            return DocumentType.Markdown;
        }
        return CodeExtensions.Contains(ext, StringComparer.OrdinalIgnoreCase) ? DocumentType.Code : DocumentType.Documentation;
    }

    private sealed record TreeResponse(
        [property: JsonPropertyName("tree")] List<TreeEntry>? Tree,
        [property: JsonPropertyName("truncated")] bool Truncated = false);

    private sealed record TreeEntry(
        [property: JsonPropertyName("path")] string? Path,
        [property: JsonPropertyName("type")] string? Type);
}
