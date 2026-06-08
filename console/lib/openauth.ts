/**
 * OpenAuth (BYO) integration helpers.
 *
 * SST OpenAuth (`@openauthjs/openauth` + `sst.aws.Auth`) issues a standards OAuth
 * 2.0 authorization-code flow against an issuer URL. The console drives it with
 * plain `fetch` against the issuer's `/authorize` + `/token` endpoints so it has
 * no hard build-time dependency on the OpenAuth client SDK — the same flow works
 * for the Smoo-identity (`smoo`) issuer, which is OIDC-compatible.
 *
 * Flow:
 *   1. `/login` → redirect the browser to `${issuer}/authorize` (code flow).
 *   2. The issuer authenticates the user and redirects back to
 *      `${redirectUri}` with `?code=...`.
 *   3. `/auth/callback` exchanges the code at `${issuer}/token` for the
 *      OpenAuth-issued JWT (`access_token`). That JWT carries `sub` / `org` /
 *      `role` and becomes the admin-API bearer.
 *
 * For a production deploy, install `@openauthjs/openauth` and swap these helpers
 * for `createClient({ clientID, issuer }).authorize(...)` / `.exchange(...)` —
 * the wire contract is identical. The hand-rolled version keeps the console
 * buildable without the SDK and documents exactly what OpenAuth provides.
 */

import { getConfig } from './config';
import type { Principal, Role } from './types';

/** Build the issuer authorize URL for the code flow. */
export function buildAuthorizeUrl(redirectUri: string, state: string): string {
    const cfg = getConfig();
    if (!cfg.openauthIssuer) {
        throw new Error('OPENAUTH_ISSUER is not configured');
    }
    const url = new URL('/authorize', cfg.openauthIssuer);
    url.searchParams.set('client_id', cfg.openauthClientId);
    url.searchParams.set('redirect_uri', redirectUri);
    url.searchParams.set('response_type', 'code');
    url.searchParams.set('state', state);
    return url.toString();
}

/** Exchange an authorization `code` for the OpenAuth-issued access token (JWT). */
export async function exchangeCode(code: string, redirectUri: string): Promise<string> {
    const cfg = getConfig();
    if (!cfg.openauthIssuer) {
        throw new Error('OPENAUTH_ISSUER is not configured');
    }
    const res = await fetch(new URL('/token', cfg.openauthIssuer).toString(), {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({
            grant_type: 'authorization_code',
            client_id: cfg.openauthClientId,
            code,
            redirect_uri: redirectUri,
        }),
    });
    if (!res.ok) {
        throw new Error(`OpenAuth token exchange failed: ${res.status}`);
    }
    const body = (await res.json()) as { access_token?: string };
    if (!body.access_token) {
        throw new Error('OpenAuth token response missing access_token');
    }
    return body.access_token;
}

/**
 * Decode the (already-verified-by-the-issuer) JWT payload for a display hint.
 *
 * This does NOT verify the signature — the admin API re-verifies the JWT on
 * every request via its `JwtVerifier`, which is the real trust boundary. The
 * decode here only extracts `sub` / `org` / `role` / `name` so the UI can render
 * the signed-in identity without an extra `/admin/me` round trip at login time.
 */
export function decodePrincipalHint(jwt: string): Principal | undefined {
    const parts = jwt.split('.');
    if (parts.length !== 3) return undefined;
    try {
        const payload = JSON.parse(Buffer.from(parts[1], 'base64url').toString('utf8')) as Record<string, unknown>;
        const role = String(payload.role ?? 'basic').toLowerCase();
        return {
            userId: String(payload.sub ?? ''),
            orgId: String(payload.org ?? payload.org_id ?? ''),
            role: (['basic', 'curator', 'admin'].includes(role) ? role : 'basic') as Role,
            displayName: payload.name ? String(payload.name) : undefined,
        };
    } catch {
        return undefined;
    }
}
