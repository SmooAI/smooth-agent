// A System.Text.Json enum converter that honors [EnumMember(Value = "...")] for
// (de)serialization. The built-in JsonStringEnumConverter<T> in net8 ignores
// [EnumMember] and uses the C# identifier instead, which would break wire values
// like "ai-agent" / "human-agent" / "in_progress" emitted by the spec enums.
//
// The generated types reference this converter so kebab/snake-cased wire enum
// values round-trip faithfully.

using System.Collections.Concurrent;
using System.Reflection;
using System.Runtime.Serialization;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace SmooAI.SmoothAgent.Generated;

public sealed class EnumMemberStringConverter<T> : JsonConverter<T> where T : struct, Enum
{
    private static readonly ConcurrentDictionary<T, string> ToWire = new();
    private static readonly Dictionary<string, T> FromWire = new(StringComparer.Ordinal);

    static EnumMemberStringConverter()
    {
        foreach (var field in typeof(T).GetFields(BindingFlags.Public | BindingFlags.Static))
        {
            var value = (T)field.GetValue(null)!;
            var wire = field.GetCustomAttribute<EnumMemberAttribute>()?.Value ?? field.Name;
            ToWire[value] = wire;
            FromWire[wire] = value;
            // Also accept the C# identifier as an alias for robustness.
            FromWire.TryAdd(field.Name, value);
        }
    }

    public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        var raw = reader.GetString();
        if (raw is not null && FromWire.TryGetValue(raw, out var value))
            return value;
        throw new JsonException($"Unknown {typeof(T).Name} value: \"{raw}\".");
    }

    public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
    {
        writer.WriteStringValue(ToWire.TryGetValue(value, out var wire) ? wire : value.ToString());
    }
}
