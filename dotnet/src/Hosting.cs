// DI wiring for smooth-operator-agent's MEAI facade.
//
// services.AddSmoothAgent(o => { o.Url = ...; o.AgentId = ...; }) registers the
// transport + SmoothAgentClient + SmoothAgentChatClient (as IChatClient), so a
// .NET host resolves an IChatClient and talks to the remote agent through the
// standard MEAI abstraction. DI-first per docs/DOTNET.md borrow-list #4.

using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;

namespace SmooAI.SmoothOperatorAgent;

/// <summary><see cref="IServiceCollection"/> extensions for registering the agent client + MEAI facade.</summary>
public static class SmoothAgentServiceCollectionExtensions
{
    /// <summary>
    /// Register the smooth-operator-agent client and its <see cref="IChatClient"/>
    /// facade. Resolves:
    /// <list type="bullet">
    ///   <item><see cref="SmoothAgentOptions"/> (singleton, configured)</item>
    ///   <item><see cref="SmoothAgentClient"/> (singleton, over the configured transport)</item>
    ///   <item><see cref="SmoothAgentChatClient"/> and <see cref="IChatClient"/> (the same singleton facade)</item>
    /// </list>
    /// The client is constructed but not connected — call
    /// <see cref="SmoothAgentClient.ConnectAsync"/> during host startup, or inject a
    /// transport that is already open (tests do the latter).
    /// </summary>
    public static IServiceCollection AddSmoothAgent(this IServiceCollection services, Action<SmoothAgentOptions> configure)
    {
        ArgumentNullException.ThrowIfNull(services);
        ArgumentNullException.ThrowIfNull(configure);

        var options = new SmoothAgentOptions();
        configure(options);

        services.TryAddSingleton(options);
        services.TryAddSingleton(sp => new SmoothAgentClient(sp.GetRequiredService<SmoothAgentOptions>().ToClientOptions()));
        services.TryAddSingleton(sp => new SmoothAgentChatClient(
            sp.GetRequiredService<SmoothAgentClient>(),
            sp.GetRequiredService<SmoothAgentOptions>()));
        services.TryAddSingleton<IChatClient>(sp => sp.GetRequiredService<SmoothAgentChatClient>());

        return services;
    }
}
