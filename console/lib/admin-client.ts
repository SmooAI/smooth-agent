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
    ConversationMessagesResponse,
    ConversationsResponse,
    DocumentSetsResponse,
    IndexingRunsResponse,
    AdminErrorBody,
    Principal,
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

    private async get<T>(path: string): Promise<T> {
        const res = await this.fetchImpl(`${this.baseUrl}${path}`, {
            headers: { Authorization: `Bearer ${this.token}` },
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

        return (await res.json()) as T;
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
}
