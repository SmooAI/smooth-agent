using System.Reflection;

namespace SmooAI.SmoothAgent.Tests;

/// <summary>Locates the repo's spec/ directory from the test assembly location.</summary>
internal static class SpecPaths
{
    public static string SpecDir { get; } = FindSpecDir();

    private static string FindSpecDir()
    {
        var dir = new DirectoryInfo(Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location)!);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "spec");
            if (Directory.Exists(candidate) && File.Exists(Path.Combine(candidate, "envelope.schema.json")))
                return candidate;
            dir = dir.Parent;
        }
        throw new DirectoryNotFoundException("Could not locate the spec/ directory above the test assembly.");
    }
}
