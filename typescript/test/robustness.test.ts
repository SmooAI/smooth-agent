/**
 * Robustness regressions found in an adversarial review of the TS client:
 *
 *  - Bug 5: an in-flight async-iterator `next()` swallowed the terminal error — a
 *    pure `for await` consumer got an indistinguishable `{ done: true }` instead of
 *    the thrown ProtocolError. It must now throw.
 *  - Bug 2: a streaming turn whose server accepts `send_message` but never emits a
 *    terminal `eventual_response` / `error` hung forever. It must now reject with a
 *    TurnTimeoutError within the bound.
 *  - Bug 4: WebSocketTransport.connect() leaked a previous non-OPEN socket (orphaned
 *    listeners double-dispatching) and had no connect timeout. It must close+detach
 *    the prior socket and time out a half-open dial.
 */
import { describe, expect, it, vi } from 'vitest';
import { SmoothAgentClient, ProtocolError, TurnTimeoutError } from '../src/client.js';
import { WebSocketTransport, type Transport, type TransportState, type WebSocketLike } from '../src/transport.js';
import type { ServerEvent } from '../src/types.js';

/** In-memory transport: captures sent frames, lets the test inject server events. */
class MockTransport implements Transport {
    state: TransportState = 'closed';
    readonly sent: string[] = [];
    private messageHandlers = new Set<(data: string) => void>();
    private closeHandlers = new Set<(info: { code?: number; reason?: string }) => void>();
    private errorHandlers = new Set<(err: unknown) => void>();

    connect(): Promise<void> {
        this.state = 'open';
        return Promise.resolve();
    }
    send(data: string): void {
        if (this.state !== 'open') throw new Error(`not open: ${this.state}`);
        this.sent.push(data);
    }
    close(): void {
        this.state = 'closed';
        for (const h of this.closeHandlers) h({ code: 1000 });
    }
    onMessage(handler: (data: string) => void): () => void {
        this.messageHandlers.add(handler);
        return () => this.messageHandlers.delete(handler);
    }
    onClose(handler: (info: { code?: number; reason?: string }) => void): () => void {
        this.closeHandlers.add(handler);
        return () => this.closeHandlers.delete(handler);
    }
    onError(handler: (err: unknown) => void): () => void {
        this.errorHandlers.add(handler);
        return () => this.errorHandlers.delete(handler);
    }
    emit(event: ServerEvent): void {
        const data = JSON.stringify(event);
        for (const h of this.messageHandlers) h(data);
    }
    lastSent<T = Record<string, unknown>>(): T {
        return JSON.parse(this.sent.at(-1)!) as T;
    }
}

function makeClient(opts?: { turnTimeout?: number }): { client: SmoothAgentClient; transport: MockTransport } {
    const transport = new MockTransport();
    let counter = 0;
    const client = new SmoothAgentClient({
        url: 'wss://test',
        transport,
        generateRequestId: () => `req-test-${++counter}`,
        requestTimeout: 1000,
        turnTimeout: opts?.turnTimeout ?? 0,
    });
    return { client, transport };
}

describe('Bug 5 — a parked iterator next() surfaces the terminal error (not silent done)', () => {
    it('throws when the turn aborts (transport disconnect) while next() is parked', async () => {
        const { client, transport } = makeClient();
        await client.connect();
        const turn = client.sendMessage({ sessionId: 's', message: 'work' });

        // Start iterating *first* so next() is parked: queue is empty, turn not done.
        // This is the exact path the bug lived in — finish() ran with a parked waiter
        // and no event to deliver, so the old code resolved { done: true } and the
        // `for await` loop ended cleanly, swallowing the failure.
        let thrown: unknown = null;
        const iterate = (async () => {
            try {
                for await (const _ of turn) {
                    // no events arrive before the abort
                }
            } catch (err) {
                thrown = err;
            }
        })();

        // Let the iterator park on next().
        await Promise.resolve();
        await Promise.resolve();

        // Transport drops → failAll → turn.abort(Error) finishes the turn with a parked
        // waiter and no delivered event.
        transport.close();

        await iterate;
        expect(thrown).toBeInstanceOf(Error);
        expect((thrown as Error).message).toMatch(/Transport closed/);

        // Awaiting the turn rejects too (both consumption styles agree).
        await expect(turn).rejects.toBeInstanceOf(Error);
    });

    it('an error event mid-stream still throws through iteration', async () => {
        const { client, transport } = makeClient();
        await client.connect();
        const turn = client.sendMessage({ sessionId: 's', message: 'boom' });
        const reqId = transport.lastSent<{ requestId: string }>().requestId;

        const seen: string[] = [];
        let thrown: unknown = null;
        const iterate = (async () => {
            try {
                for await (const ev of turn) seen.push(ev.type);
            } catch (err) {
                thrown = err;
            }
        })();

        await Promise.resolve();
        transport.emit({ type: 'stream_token', requestId: reqId, token: 'A', data: { requestId: reqId, token: 'A' } });
        transport.emit({
            type: 'error',
            requestId: reqId,
            data: { requestId: reqId, error: { code: 'RATE_LIMITED', message: 'slow down' } },
        });

        await iterate;
        expect(seen).toContain('stream_token');
        expect(thrown).toBeInstanceOf(ProtocolError);
        expect((thrown as ProtocolError).code).toBe('RATE_LIMITED');
    });
});

describe('Bug 2 — streaming turn times out instead of hanging', () => {
    it('rejects the turn with a TurnTimeoutError when no terminal event arrives', async () => {
        const { client, transport } = makeClient({ turnTimeout: 50 });
        await client.connect();
        const turn = client.sendMessage({ sessionId: 's', message: 'hang' });
        const reqId = transport.lastSent<{ requestId: string }>().requestId;

        // An intermediate event arrives, but the server never sends a terminal one.
        transport.emit({ type: 'stream_token', requestId: reqId, token: 'partial', data: { requestId: reqId, token: 'partial' } });

        const start = Date.now();
        await expect(turn).rejects.toBeInstanceOf(TurnTimeoutError);
        await expect(turn).rejects.toMatchObject({ requestId: reqId });
        // Resolved well within a generous bound (the timeout is 50ms).
        expect(Date.now() - start).toBeLessThan(2000);
    });

    it('a `for await` consumer also sees the timeout thrown', async () => {
        const { client, transport } = makeClient({ turnTimeout: 50 });
        await client.connect();
        const turn = client.sendMessage({ sessionId: 's', message: 'hang' });

        let thrown: unknown = null;
        try {
            for await (const _ of turn) {
                // never reaches a terminal event
            }
        } catch (err) {
            thrown = err;
        }
        expect(thrown).toBeInstanceOf(TurnTimeoutError);
    });

    it('does not fire the timeout when a terminal event arrives first', async () => {
        const { client, transport } = makeClient({ turnTimeout: 50 });
        await client.connect();
        const turn = client.sendMessage({ sessionId: 's', message: 'ok' });
        const reqId = transport.lastSent<{ requestId: string }>().requestId;

        transport.emit({
            type: 'eventual_response',
            requestId: reqId,
            status: 200,
            data: { requestId: reqId, status: 200, data: { messageId: 'm', response: null } },
        });

        const final = await turn;
        expect(final.type).toBe('eventual_response');
        // Wait past the timeout window; the turn must stay resolved, not flip to error.
        await new Promise((r) => setTimeout(r, 80));
        await expect(turn).resolves.toMatchObject({ type: 'eventual_response' });
    });
});

// ── Bug 4: fake WebSocket for transport reconnect / connect-timeout tests ──────────
const WS_CONNECTING = 0;
const WS_OPEN = 1;
const WS_CLOSED = 3;

class FakeWebSocket implements WebSocketLike {
    static instances: FakeWebSocket[] = [];
    readyState = WS_CONNECTING;
    closed = false;
    private listeners: Record<string, ((ev: any) => void)[]> = {};

    constructor(public url: string) {
        FakeWebSocket.instances.push(this);
    }
    send(_data: string): void {}
    close(): void {
        if (this.closed) return;
        this.closed = true;
        this.readyState = WS_CLOSED;
        this.dispatch('close', { code: 1000 });
    }
    addEventListener(type: string, listener: (ev: any) => void): void {
        (this.listeners[type] ??= []).push(listener);
    }
    private dispatch(type: string, ev: any): void {
        for (const l of this.listeners[type] ?? []) l(ev);
    }
    // test drivers
    open(): void {
        this.readyState = WS_OPEN;
        this.dispatch('open', {});
    }
    fail(): void {
        this.dispatch('error', new Error('dial failed'));
    }
    message(data: string): void {
        this.dispatch('message', { data });
    }
}

describe('Bug 4 — transport reconnect does not leak the previous socket', () => {
    it('closes and detaches a prior non-OPEN socket before dialing, so it cannot double-dispatch', async () => {
        FakeWebSocket.instances = [];
        const transport = new WebSocketTransport('wss://x', (u) => new FakeWebSocket(u), 0 /* no connect timeout */);

        const messages: string[] = [];
        transport.onMessage((d) => messages.push(d));

        // First dial fails (never reaches OPEN), leaving a non-OPEN socket behind.
        const first = transport.connect();
        FakeWebSocket.instances[0]!.fail();
        await expect(first).rejects.toBeInstanceOf(Error);

        const staleSocket = FakeWebSocket.instances[0]!;

        // Second dial: the stale socket must have been closed + detached.
        const second = transport.connect();
        const liveSocket = FakeWebSocket.instances[1]!;
        liveSocket.open();
        await second;

        expect(staleSocket.closed).toBe(true);

        // A late message from the stale socket must be ignored (it was detached).
        staleSocket.message('{"from":"stale"}');
        // A message from the live socket is delivered.
        liveSocket.message('{"from":"live"}');

        expect(messages).toEqual(['{"from":"live"}']);
    });

    it('times out a half-open dial and tears down the socket', async () => {
        FakeWebSocket.instances = [];
        const transport = new WebSocketTransport('wss://x', (u) => new FakeWebSocket(u), 40 /* 40ms connect timeout */);

        const start = Date.now();
        // Never call .open() — the dial hangs.
        await expect(transport.connect()).rejects.toThrow(/timed out/);
        expect(Date.now() - start).toBeLessThan(2000);

        // The half-open socket was closed and the transport is back to 'closed'.
        expect(FakeWebSocket.instances[0]!.closed).toBe(true);
        expect(transport.state).toBe('closed');
    });
});
