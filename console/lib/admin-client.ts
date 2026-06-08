/**
 * Typed fetch client for the `smooth-operator` admin API (`/admin/*`).
 *
 * Every method issues an `Authorization: Bearer <token>` request against the
 * configured base URL and parses the JSON into the shapes declared in
 * `./types.ts`. Auth/handler failures (the protocol `error` envelope) are
 * surfaced as a typed {@link AdminApiError} carrying the HTTP status + code, so
 * UI code can branch on 401/403/404 without re-parsing the body.
 *
 * The base URL + bearer token come from the caller (a server component reads
 * them from the session / env), so this module is environment-agnostic and runs
 * the same in a server component, a route handler, or a test.
 */

import type {
    AgentSettings,
    ConnectorConfig,
    ConnectorsResponse,
    ConnectorWrite,
    ConversationMessagesResponse,
    ConversationsResponse,
    DocumentSetsResponse,
    IndexingRun,
    IndexingRunsResponse,
    AdminErrorBody,
    Principal,
    SettingsWrite,
} from './types';

/** A typed error carrying the admin API's status + protocol error code. */
export class AdminApiError extends Error {
    constructor(
        public readonly status: number,
        public readonly code: string,
        message: string,
    ) {
        super(message);
        this.name = 'AdminApiError';
    }

    get isUnauthorized() {
        return this.status === 401;
    }
    get isForbidden() {
        return this.status === 403;
    }
    get isNotFound() {
        return this.status === 404;
    }
}

export interface AdminClientOptions {
    /** Admin API base URL, e.g. `http://127.0.0.1:8840`. No trailing slash. */
    baseUrl: string;
    /**
     * Bearer token. For `AUTH_MODE=none` dev servers any non-empty value works
     * (the server returns a fixed Admin principal); the dev console sends `dev`.
     */
    token: string;
    /** Optional fetch override (tests). */
    fetchImpl?: typeof fetch;
}

export class AdminClient {
    private readonly baseUrl: string;
    private readonly token: string;
    private readonly fetchImpl: typeof fetch;

    constructor(opts: AdminClientOptions) {
        this.baseUrl = opts.baseUrl.replace(/\/+$/, '');
        this.token = opts.token;
        this.fetchImpl = opts.fetchImpl ?? fetch;
    }

    /**
     * Issue an authenticated request and parse the JSON body. A non-2xx surfaces
     * as a typed {@link AdminApiError} carrying the protocol error code/message.
     * `parse: false` skips JSON parsing for empty-body responses (e.g. `204`).
     */
    private async request<T>(
        method: 'GET' | 'POST' | 'PUT' | 'DELETE',
        path: string,
        opts: { body?: unknown; parse?: boolean } = {},
    ): Promise<T> {
        const headers: Record<string, string> = { Authorization: `Bearer ${this.token}` };
        if (opts.body !== undefined) headers['Content-Type'] = 'application/json';

        const res = await this.fetchImpl(`${this.baseUrl}${path}`, {
            method,
            headers,
            body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
            // Always hit the live admin API — never cache a stale management read.
            cache: 'no-store',
        });

        if (!res.ok) {
            let code = 'HTTP_ERROR';
            let message = `${res.status} ${res.statusText}`;
            try {
                const body = (await res.json()) as AdminErrorBody;
                const err = body.error ?? body;
                if (err.code) code = err.code;
                if (err.message) message = err.message;
            } catch {
                // non-JSON body — keep the status-line message.
            }
            throw new AdminApiError(res.status, code, message);
        }

        if (opts.parse === false) return undefined as T;
        return (await res.json()) as T;
    }

    private get<T>(path: string): Promise<T> {
        return this.request<T>('GET', path);
    }

    /** `GET /admin/health` — liveness (no auth required). */
    async health(): Promise<{ status: string }> {
        const res = await this.fetchImpl(`${this.baseUrl}/admin/health`, { cache: 'no-store' });
        if (!res.ok) {
            throw new AdminApiError(res.status, 'HTTP_ERROR', `${res.status} ${res.statusText}`);
        }
        return (await res.json()) as { status: string };
    }

    /** `GET /admin/me` — the caller's principal. */
    me(): Promise<Principal> {
        return this.get<Principal>('/admin/me');
    }

    /** `GET /admin/conversations?limit&cursor` — org-scoped chat history. */
    conversations(params: { limit?: number; cursor?: number } = {}): Promise<ConversationsResponse> {
        const q = new URLSearchParams();
        if (params.limit != null) q.set('limit', String(params.limit));
        if (params.cursor != null) q.set('cursor', String(params.cursor));
        const qs = q.toString();
        return this.get<ConversationsResponse>(`/admin/conversations${qs ? `?${qs}` : ''}`);
    }

    /** `GET /admin/conversations/{id}/messages` — messages for one conversation. */
    conversationMessages(id: string): Promise<ConversationMessagesResponse> {
        return this.get<ConversationMessagesResponse>(`/admin/conversations/${encodeURIComponent(id)}/messages`);
    }

    /** `GET /admin/indexing/runs` — indexing-run status (Curator+). */
    indexingRuns(): Promise<IndexingRunsResponse> {
        return this.get<IndexingRunsResponse>('/admin/indexing/runs');
    }

    /** `GET /admin/document-sets` — document set names + counts (Curator+). */
    documentSets(): Promise<DocumentSetsResponse> {
        return this.get<DocumentSetsResponse>('/admin/document-sets');
    }

    // --- Connector config (Phase 12, increment 3 write API) -----------------

    /** `GET /admin/connectors` — list this org's connector configs (Curator+). */
    listConnectors(): Promise<ConnectorsResponse> {
        return this.get<ConnectorsResponse>('/admin/connectors');
    }

    /** `GET /admin/connectors/{id}` — one connector (Curator+; 404 cross-org/unknown). */
    async getConnector(id: string): Promise<ConnectorConfig> {
        const res = await this.get<{ connector: ConnectorConfig }>(`/admin/connectors/${encodeURIComponent(id)}`);
        return res.connector;
    }

    /** `POST /admin/connectors` — create a connector (Admin). 400 on validation error. */
    async createConnector(body: ConnectorWrite): Promise<ConnectorConfig> {
        const res = await this.request<{ connector: ConnectorConfig }>('POST', '/admin/connectors', { body });
        return res.connector;
    }

    /** `PUT /admin/connectors/{id}` — update a connector (Admin). 400 on validation error. */
    async updateConnector(id: string, body: ConnectorWrite): Promise<ConnectorConfig> {
        const res = await this.request<{ connector: ConnectorConfig }>('PUT', `/admin/connectors/${encodeURIComponent(id)}`, { body });
        return res.connector;
    }

    /** `DELETE /admin/connectors/{id}` — delete a connector (Admin; 204 / 404). */
    deleteConnector(id: string): Promise<void> {
        return this.request<void>('DELETE', `/admin/connectors/${encodeURIComponent(id)}`, { parse: false });
    }

    /** `POST /admin/connectors/{id}/index` — build + run one indexing pass (Curator+). */
    async indexConnector(id: string): Promise<IndexingRun> {
        const res = await this.request<{ run: IndexingRun }>('POST', `/admin/connectors/${encodeURIComponent(id)}/index`, { body: {} });
        return res.run;
    }

    // --- Agent settings -----------------------------------------------------

    /** `GET /admin/settings` — the org's agent settings (defaults if unset; Curator+). */
    async getSettings(): Promise<AgentSettings> {
        const res = await this.get<{ settings: AgentSettings }>('/admin/settings');
        return res.settings;
    }

    /** `PUT /admin/settings` — replace the org's agent settings (Admin). */
    async putSettings(body: SettingsWrite): Promise<AgentSettings> {
        const res = await this.request<{ settings: AgentSettings }>('PUT', '/admin/settings', { body });
        return res.settings;
    }
}
