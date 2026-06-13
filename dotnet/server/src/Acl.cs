using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server;

/// <summary>
/// Who may read a document. Mirrors the Rust <c>DocAcl</c>: public, or restricted to a set of
/// entitlement groups (e.g. <c>github:owner/repo</c>) that must intersect the caller's groups.
/// </summary>
public sealed record DocumentAcl(bool Public, IReadOnlyList<string> Groups)
{
    public static DocumentAcl PublicAcl { get; } = new(true, Array.Empty<string>());

    public static DocumentAcl ForGroups(params string[] groups) => new(false, groups);

    /// <summary>True if <paramref name="access"/> is permitted to read. Fail-closed for private docs.</summary>
    public bool AllowsAccess(AccessContext access)
    {
        if (Public)
        {
            return true;
        }
        foreach (var group in Groups)
        {
            if (access.Groups.Contains(group, StringComparer.Ordinal))
            {
                return true;
            }
        }
        return false;
    }
}

/// <summary>
/// An ACL-aware in-process knowledge store: documents carry a <see cref="DocumentAcl"/>, and
/// retrieval filters by the caller's <see cref="AccessContext"/> BEFORE scoring — so a private
/// document is never even a candidate for an unentitled user. The C# analog of the Rust
/// <c>knowledge_for_access</c> seam that closed the #1 adversarial leak (private repo docs
/// retrievable by any chat user).
/// </summary>
public sealed class AclKnowledgeStore
{
    private readonly object _gate = new();
    private readonly List<Entry> _entries = new();

    public Task IngestAsync(KnowledgeDocument document, DocumentAcl acl, CancellationToken cancellationToken = default)
    {
        lock (_gate)
        {
            _entries.RemoveAll(e => e.Document.Id == document.Id);
            _entries.Add(new Entry(document, acl));
        }
        return Task.CompletedTask;
    }

    /// <summary>Retrieve the top hits the caller is entitled to read.</summary>
    public Task<IReadOnlyList<KnowledgeResult>> QueryForAccessAsync(string query, int limit, AccessContext access, CancellationToken cancellationToken = default)
    {
        lock (_gate)
        {
            IReadOnlyList<KnowledgeResult> hits = _entries
                .Where(e => e.Acl.AllowsAccess(access)) // ACL filter FIRST — fail-closed
                .Select(e => new KnowledgeResult(e.Document.Id, e.Document.Content, Score(query, e.Document.Content), e.Document.Source))
                .Where(r => r.Score > 0)
                .OrderByDescending(r => r.Score)
                .Take(limit)
                .ToList();
            return Task.FromResult(hits);
        }
    }

    private static double Score(string query, string content)
    {
        var queryTokens = Tokenize(query);
        if (queryTokens.Count == 0)
        {
            return 0;
        }
        var contentTokens = Tokenize(content);
        return queryTokens.Count(contentTokens.Contains);
    }

    private static HashSet<string> Tokenize(string text)
    {
        var tokens = new HashSet<string>(StringComparer.Ordinal);
        foreach (var raw in text.ToLowerInvariant().Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries))
        {
            var token = new string(raw.Where(char.IsLetterOrDigit).ToArray());
            if (token.Length > 2)
            {
                tokens.Add(token);
            }
        }
        return tokens;
    }

    private readonly record struct Entry(KnowledgeDocument Document, DocumentAcl Acl);
}
