/**
 * Compile-time assertions that the generated types accept the conformance
 * fixtures' shapes and that the discriminated unions narrow correctly.
 *
 * These are checked by `tsc` (run via the `test` script's typecheck pass and the
 * `typecheck` script). The runtime assertions below also exercise the guards so
 * vitest reports a passing test, but the load-bearing checks are the type
 * annotations — if the generated types drift, this file stops compiling.
 */
import { describe, expect, it } from 'vitest';
import type {
    CreateConversationSessionRequest,
    CreateConversationSessionResponse,
    SendMessageRequest,
    StreamChunk,
    EventualResponse,
    ServerEvent,
    ClientAction,
} from '../src/index.js';
import { isServerEvent, isClientAction, isEvent } from '../src/index.js';

// ── Type-level: fixtures must be assignable to the generated interfaces ──────

const createReq: CreateConversationSessionRequest = {
    action: 'create_conversation_session',
    requestId: 'req-a1b2c3d4-0001',
    agentId: '11111111-1111-1111-1111-111111111111',
    userName: 'Alice',
    userEmail: 'alice@example.com',
    browserFingerprint: 'fp_abc123def456',
    metadata: { campaignSource: 'homepage-chat-widget', planTier: 'pro' },
};

const createResp: CreateConversationSessionResponse = {
    sessionId: '22222222-2222-2222-2222-222222222222',
    conversationId: '33333333-3333-3333-3333-333333333333',
    agentId: '11111111-1111-1111-1111-111111111111',
    agentName: 'Aria',
    userParticipantId: '44444444-4444-4444-4444-444444444444',
    agentParticipantId: '55555555-5555-5555-5555-555555555555',
};

const sendReq: SendMessageRequest = {
    action: 'send_message',
    requestId: 'req-a1b2c3d4-0002',
    sessionId: '22222222-2222-2222-2222-222222222222',
    message: 'What is the status of my last order?',
    stream: true,
};

const streamChunk: StreamChunk = {
    type: 'stream_chunk',
    requestId: 'req-a1b2c3d4-0002',
    node: 'knowledge_search',
    data: {
        requestId: 'req-a1b2c3d4-0002',
        node: 'knowledge_search',
        state: {
            rawResponse: null,
            structuredResponse: { snippets: ['Order #ORD-9982 shipped.'] },
            pendingWriteConfirmation: null,
            pendingOtpVerification: null,
        },
        done: false,
    },
    timestamp: 1749340800000,
};

const eventual: EventualResponse = {
    type: 'eventual_response',
    requestId: 'req-a1b2c3d4-0002',
    status: 200,
    data: {
        requestId: 'req-a1b2c3d4-0002',
        status: 200,
        data: {
            messageId: '66666666-6666-6666-6666-666666666666',
            response: { responseParts: ['shipped'] },
            needsEscalation: false,
        },
    },
    timestamp: 1749340803500,
};

// Discriminated-union membership (compile-time).
const _action: ClientAction = sendReq;
const _event: ServerEvent = streamChunk;

// Narrowing: inside this branch, `ev` must be `StreamChunk` so `.data.node` exists.
function narrow(ev: ServerEvent): string | undefined {
    if (ev.type === 'stream_chunk') return ev.data.node;
    if (ev.type === 'eventual_response') return ev.data.data.messageId;
    return undefined;
}

describe('type-level fixture conformance', () => {
    it('fixtures are assignable to generated types and guards agree', () => {
        // Runtime exercise so vitest registers a pass; the real check is that this
        // file type-checks at all.
        expect(createReq.action).toBe('create_conversation_session');
        expect(createResp.agentName).toBe('Aria');
        expect(isClientAction(_action)).toBe(true);
        expect(isServerEvent(_event)).toBe(true);
        expect(isEvent(streamChunk, 'stream_chunk')).toBe(true);
        expect(isEvent(streamChunk, 'eventual_response')).toBe(false);
        expect(narrow(streamChunk)).toBe('knowledge_search');
        expect(narrow(eventual)).toBe('66666666-6666-6666-6666-666666666666');
    });
});
