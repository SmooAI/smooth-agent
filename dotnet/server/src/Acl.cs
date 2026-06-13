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
/// Supplies an <see cref="IKnowledgeBase"/> scoped to a caller's <see cref="AccessContext"/> — the
/// C# analog of the Rust <c>storage.knowledge_for_access(&amp;access)</c> seam. The turn runner reads
/// retrieval through this so ACL is enforced on the live chat path, not just at ingest.
/// </summary>
public interface IAccessKnowledge
{
    /// <summary>A knowledge handle that only returns documents <paramref name="access"/> may read.</summary>
    IKnowledgeBase? ForAccess(AccessContext access);
}

/// <summary>
/// Wraps a plain <see cref="IKnowledgeBase"/> with no ACL filtering — for deployments that don't
/// use per-document access control (every doc is org-public).
/// </summary>
public sealed class StaticAccessKnowledge : IAccessKnowledge
{
    private readonly IKnowledgeBase _knowledge;

    public StaticAccessKnowledge(IKnowledgeBase knowledge) => _knowledge = knowledge;

    public IKnowledgeBase? ForAccess(AccessContext access) => _knowledge;
}

/// <summary>
/// An ACL-aware in-process knowledge store: documents carry a <see cref="DocumentAcl"/>, and
/// retrieval filters by the caller's <see cref="AccessContext"/> BEFORE scoring — so a private
/// document is never even a candidate for an unentitled user. The C# analog of the Rust
/// <c>knowledge_for_access</c> seam that closed the #1 adversarial leak (private repo docs
/// retrievable by any chat user).
/// </summary>
public sealed class AclKnowledgeStore : IAccessKnowledge
{
    /// <summary>A read-only knowledge handle scoped to <paramref name="access"/> (ACL-filtered).</summary>
    public IKnowledgeBase? ForAccess(AccessContext access) => new ScopedView(this, access);

    /// <summary>
    /// A write-only knowledge handle that stamps <paramref name="acl"/> on everything ingested
    /// through it — so the ingest pipeline can feed a connector's docs in with a repo's entitlement
    /// group (e.g. <c>github:owner/repo</c>) without the pipeline knowing about ACLs.
    /// </summary>
    public IKnowledgeBase WithAcl(DocumentAcl acl) => new IngestView(this, acl);

    private sealed class IngestView : IKnowledgeBase
    {
        private readonly AclKnowledgeStore _store;
        private readonly DocumentAcl _acl;

        public IngestView(AclKnowledgeStore store, DocumentAcl acl)
        {
            _store = store;
            _acl = acl;
        }

        public Task IngestAsync(KnowledgeDocument document, CancellationToken cancellationToken = default) =>
            _store.IngestAsync(document, _acl, cancellationToken);

        public Task<IReadOnlyList<KnowledgeResult>> QueryAsync(string query, int limit, CancellationToken cancellationToken = default) =>
            throw new NotSupportedException("An ingest view is write-only; query through ForAccess(access).");
    }

    private sealed class ScopedView : IKnowledgeBase
    {
        private readonly AclKnowledgeStore _store;
        private readonly AccessContext _access;

        public ScopedView(AclKnowledgeStore store, AccessContext access)
        {
            _store = store;
            _access = access;
        }

        public Task IngestAsync(KnowledgeDocument document, CancellationToken cancellationToken = default) =>
            throw new NotSupportedException("An access-scoped knowledge view is read-only; ingest through the AclKnowledgeStore.");

        public Task<IReadOnlyList<KnowledgeResult>> QueryAsync(string query, int limit, CancellationToken cancellationToken = default) =>
            _store.QueryForAccessAsync(query, limit, _access, cancellationToken);
    }

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
