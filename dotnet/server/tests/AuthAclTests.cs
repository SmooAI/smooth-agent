using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using SmooAI.SmoothOperator.Core;
using SmooAI.SmoothOperator.Server;

namespace SmooAI.SmoothOperator.Server.Tests;

/// <summary>
/// Phase-4 parity tests: token → AccessContext resolution (jwt / trusted / fail-closed), and
/// ACL-filtered retrieval. The ACL tests mirror the Rust server's acl_chat_leak suite — the #1
/// adversarial finding (a private repo's docs must never be retrievable by an unentitled user).
/// </summary>
public class AuthAclTests
{
    private static string Base64Url(byte[] bytes) =>
        Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    private static string TrustedToken(object claims) =>
        Base64Url(Encoding.UTF8.GetBytes(JsonSerializer.Serialize(claims)));

    private static string Hs256Jwt(object claims, string secret)
    {
        var header = Base64Url(Encoding.UTF8.GetBytes("""{"alg":"HS256","typ":"JWT"}"""));
        var payload = Base64Url(Encoding.UTF8.GetBytes(JsonSerializer.Serialize(claims)));
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(secret));
        var signature = Base64Url(hmac.ComputeHash(Encoding.ASCII.GetBytes($"{header}.{payload}")));
        return $"{header}.{payload}.{signature}";
    }

    // ───────────────────────────── auth resolution ─────────────────────────────

    [Fact]
    public void NoToken_ResolvesAnonymous()
    {
        var resolver = new TokenAccessResolver(new AuthOptions { Mode = AuthMode.Jwt, Hs256Secret = "s" });
        var access = resolver.Resolve(null);
        Assert.True(access.IsAnonymous);
        Assert.Empty(access.Groups);
    }

    [Fact]
    public void TrustedMode_DecodesIdentity()
    {
        var resolver = new TokenAccessResolver(new AuthOptions { Mode = AuthMode.Trusted });
        var token = TrustedToken(new { sub = "u1", org = "acme", role = "curator", groups = new[] { "github:acme/private" } });

        var access = resolver.Resolve(token);

        Assert.False(access.IsAnonymous);
        Assert.Equal("u1", access.Principal.Sub);
        Assert.Equal("curator", access.Principal.Role);
        Assert.Contains("github:acme/private", access.Groups);
    }

    [Fact]
    public void JwtMode_ValidSignature_ResolvesPrincipal()
    {
        const string secret = "super-secret-key";
        var resolver = new TokenAccessResolver(new AuthOptions { Mode = AuthMode.Jwt, Hs256Secret = secret });
        var token = Hs256Jwt(new { sub = "u1", org = "acme", role = "basic", groups = new[] { "TS-Eng" } }, secret);

        var access = resolver.Resolve(token);

        Assert.False(access.IsAnonymous);
        Assert.Equal("u1", access.Principal.Sub);
        Assert.Contains("TS-Eng", access.Groups);
    }

    [Fact]
    public void JwtMode_BadSignature_FailsClosedToAnonymous()
    {
        var resolver = new TokenAccessResolver(new AuthOptions { Mode = AuthMode.Jwt, Hs256Secret = "the-real-secret" });
        var forged = Hs256Jwt(new { sub = "attacker", groups = new[] { "github:acme/private" } }, "a-different-secret");

        var access = resolver.Resolve(forged);

        Assert.True(access.IsAnonymous);
        Assert.Empty(access.Groups);
    }

    [Fact]
    public void JwtMode_Expired_FailsClosed()
    {
        const string secret = "s";
        var resolver = new TokenAccessResolver(new AuthOptions { Mode = AuthMode.Jwt, Hs256Secret = secret });
        var expired = Hs256Jwt(new { sub = "u1", groups = new[] { "g" }, exp = DateTimeOffset.UtcNow.AddMinutes(-5).ToUnixTimeSeconds() }, secret);

        Assert.True(resolver.Resolve(expired).IsAnonymous);
    }

    [Fact]
    public void Malformed_FailsClosed()
    {
        var resolver = new TokenAccessResolver(new AuthOptions { Mode = AuthMode.Trusted });
        Assert.True(resolver.Resolve("!!!not-base64-or-json!!!").IsAnonymous);
    }

    // ───────────────────────────── ACL enforcement ─────────────────────────────

    private static AclKnowledgeStore SeededStore()
    {
        var store = new AclKnowledgeStore();
        store.IngestAsync(new KnowledgeDocument("pub", "Public hours are 9 to 5.", "public.md"), DocumentAcl.PublicAcl);
        store.IngestAsync(new KnowledgeDocument("secret", "The private launch code is hunter2.", "acme/private/launch.md"),
            DocumentAcl.ForGroups("github:acme/private"));
        return store;
    }

    private static AccessContext WithGroups(params string[] groups) =>
        new(new Principal("u", "acme", "basic", groups), IsAnonymous: groups.Length == 0);

    [Fact]
    public async Task Anonymous_SeesOnlyPublic()
    {
        var hits = await SeededStore().QueryForAccessAsync("launch code", 10, AccessContext.Anonymous);
        Assert.DoesNotContain(hits, h => h.DocumentId == "secret");
    }

    [Fact]
    public async Task EntitledUser_CanReadPrivateDoc()
    {
        var hits = await SeededStore().QueryForAccessAsync("private launch code", 10, WithGroups("github:acme/private"));
        Assert.Contains(hits, h => h.DocumentId == "secret" && h.Chunk.Contains("hunter2"));
    }

    [Fact]
    public async Task PrivateDoc_NotLeakedToUnentitledUser()
    {
        // A user authenticated, but WITHOUT the entitling group, must not retrieve the private doc.
        var hits = await SeededStore().QueryForAccessAsync("private launch code hunter2", 10, WithGroups("github:acme/other"));
        Assert.DoesNotContain(hits, h => h.DocumentId == "secret");
    }
}
