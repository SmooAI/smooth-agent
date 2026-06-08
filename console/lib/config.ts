/**
 * Console runtime configuration, read from env (server-only).
 *
 * Two auth modes:
 *
 * - **`dev`** (`CONSOLE_AUTH=dev`) — no real identity provider. A simple
 *   username form mints a local session and the admin client sends the bearer
 *   `dev` against a server running `AUTH_MODE=none` (which returns a fixed Admin
 *   principal). This is the smoke-test path and is **only** reachable when the
 *   env is set to `dev`.
 *
 * - **`openauth`** (default) — BYO SST OpenAuth (or any OIDC IdP / Smoo
 *   identity). The login route redirects to the issuer; the callback exchanges
 *   the code for the OpenAuth-issued JWT (carrying `sub` / `org` / `role`),
 *   which is stored in the session and forwarded as the admin-API bearer.
 */

export type ConsoleAuthMode = 'dev' | 'openauth';

export interface ConsoleConfig {
    /** The selected auth mode. Defaults to `openauth` (secure-by-default). */
    authMode: ConsoleAuthMode;
    /** Admin API base URL, e.g. `http://127.0.0.1:8840`. */
    adminBaseUrl: string;
    /** OpenAuth issuer base URL (only required in `openauth` mode). */
    openauthIssuer?: string;
    /** OpenAuth client id (only required in `openauth` mode). */
    openauthClientId: string;
    /** The reported auth mode of the backend (for the settings page). */
    backendAuthMode: string;
    /** The configured model / gateway (display-only, settings page). */
    model: string;
    gatewayUrl: string;
}

export function getConfig(): ConsoleConfig {
    const authMode: ConsoleAuthMode = process.env.CONSOLE_AUTH === 'dev' ? 'dev' : 'openauth';
    return {
        authMode,
        adminBaseUrl: process.env.ADMIN_API_URL ?? 'http://127.0.0.1:8840',
        openauthIssuer: process.env.OPENAUTH_ISSUER,
        openauthClientId: process.env.OPENAUTH_CLIENT_ID ?? 'smooth-console',
        backendAuthMode: process.env.BACKEND_AUTH_MODE ?? (authMode === 'dev' ? 'none' : 'jwt'),
        model: process.env.SMOOTH_AGENT_MODEL ?? 'unknown',
        gatewayUrl: process.env.SMOOTH_AGENT_GATEWAY_URL ?? 'unknown',
    };
}
