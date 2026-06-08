/**
 * TypeScript types mirroring the `smooth-operator` admin API JSON shapes.
 *
 * These match the serde output of the Rust handlers in
 * `rust/smooth-operator-server/src/admin.rs` (camelCase) and the domain types in
 * `rust/smooth-operator/src/domain.rs`. Where the shapes overlap with the
 * published `@smooai/smooth-operator` client (Conversation / Message / Citation
 * domain types) we keep the same field names so the two can be unified later;
 * for now the console declares them locally so it has no runtime dependency on
 * the client package.
 */

/** Role ordering: Admin >= Curator >= Basic. Serialized lowercase. */
export type Role = 'basic' | 'curator' | 'admin';

/** The authenticated principal — `GET /admin/me`. */
export interface Principal {
    userId: string;
    orgId: string;
    role: Role;
    displayName?: string;
}

/** A conversation row — `GET /admin/conversations`. */
export interface ConversationRow {
    id: string;
    name: string;
    platform: string;
    createdAt: string; // ISO-8601
    updatedAt: string; // ISO-8601
}

export interface ConversationsResponse {
    conversations: ConversationRow[];
    nextCursor: number | null;
}

/** A single content element within a message (`domain::ContentItem`). */
export interface ContentItem {
    type: string;
    text?: string;
}

/** Structured content of a message (`domain::MessageContent`). */
export interface MessageContent {
    items: ContentItem[];
    text?: string;
    structuredResponse?: unknown;
}

/** Abbreviated sender/recipient descriptor (`domain::ParticipantRef`). */
export interface ParticipantRef {
    id: string;
    type: string;
    name?: string;
}

export type Direction = 'inbound' | 'outbound';

/** A single message (`domain::Message`). */
export interface Message {
    id: string;
    externalId?: string;
    organizationId?: string;
    conversationId?: string;
    direction: Direction;
    content: MessageContent;
    from?: ParticipantRef;
    to?: ParticipantRef;
    metadataJson?: unknown;
    analyticsJson?: unknown;
    createdAt: string;
    updatedAt?: string;
}

export interface ConversationMessagesResponse {
    conversationId: string;
    messages: Message[];
    nextCursor: string | null;
}

export type IndexingRunStatus = 'running' | 'succeeded' | 'failed';

/** An indexing run — `GET /admin/indexing/runs`. */
export interface IndexingRun {
    id: string;
    connectorName: string;
    status: IndexingRunStatus;
    startedAt: string;
    finishedAt: string | null;
    documentsSeen: number;
    chunksIndexed: number;
    documentsSkipped: number;
    cursor: string | null;
    error: string | null;
}

export interface IndexingRunsResponse {
    runs: IndexingRun[];
}

/** A document set — `GET /admin/document-sets`. */
export interface DocumentSetRow {
    name: string;
    documentCount: number;
}

export interface DocumentSetsResponse {
    documentSets: DocumentSetRow[];
}

/** The protocol error envelope returned on auth/handler failures. */
export interface AdminErrorBody {
    error?: { code: string; message: string };
    code?: string;
    message?: string;
}
