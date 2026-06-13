using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.AspNetCore;

/// <summary>DI wiring for the smooth-operator server.</summary>
public static class ServiceCollectionExtensions
{
    /// <summary>
    /// Register the server's session store. The host must also register an <see cref="IChatClient"/>
    /// (the model), and may register an <see cref="IAccessKnowledge"/> (for ACL-scoped RAG grounding —
    /// e.g. an <see cref="AclKnowledgeStore"/>, or a <see cref="StaticAccessKnowledge"/> wrapping a
    /// plain knowledge base) and a <see cref="TokenAccessResolver"/> (to authenticate connections).
    /// The frame dispatcher itself is built per-connection by the WebSocket host (it's bound to that
    /// connection's resolved <see cref="AccessContext"/>).
    /// </summary>
    public static IServiceCollection AddSmoothOperatorServer(this IServiceCollection services)
    {
        services.TryAddSingleton<ISessionStore, InMemorySessionStore>();
        return services;
    }
}
