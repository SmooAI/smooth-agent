using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using SmooAI.SmoothOperator.Core;

namespace SmooAI.SmoothOperator.Server.AspNetCore;

/// <summary>DI wiring for the smooth-operator server.</summary>
public static class ServiceCollectionExtensions
{
    /// <summary>
    /// Register the server's session store, turn runner, and frame dispatcher. The host must also
    /// register an <see cref="IChatClient"/> (the model) and may register an
    /// <see cref="IKnowledgeBase"/> (for RAG grounding).
    /// </summary>
    public static IServiceCollection AddSmoothOperatorServer(this IServiceCollection services)
    {
        services.TryAddSingleton<ISessionStore, InMemorySessionStore>();
        services.TryAddSingleton(sp => new TurnRunner(
            sp.GetRequiredService<IChatClient>(),
            sp.GetRequiredService<ISessionStore>(),
            sp.GetService<IKnowledgeBase>()));
        services.TryAddSingleton(sp => new FrameDispatcher(
            sp.GetRequiredService<ISessionStore>(),
            sp.GetRequiredService<TurnRunner>()));
        return services;
    }
}
