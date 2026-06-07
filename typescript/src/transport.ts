/**
 * Transport abstraction for the client.
 *
 * The client is deliberately decoupled from any concrete WebSocket implementation
 * so it can be unit-tested with a mock and run on Node, the browser, or a custom
 * socket. A transport is anything that can send a string frame and surface
 * incoming string frames + lifecycle events.
 */

export type TransportState = 'connecting' | 'open' | 'closing' | 'closed';

/** Minimal injectable transport contract. Mirrors the browser `WebSocket` subset. */
export interface Transport {
    readonly state: TransportState;
    /** Send a serialized frame. Throws / rejects if the transport is not open. */
    send(data: string): void;
    /** Open the connection. Resolves once the transport reaches `open`. */
    connect(): Promise<void>;
    /** Close the connection. */
    close(code?: number, reason?: string): void;
    /** Register a handler for incoming string frames. Returns an unsubscribe fn. */
    onMessage(handler: (data: string) => void): () => void;
    /** Register a handler for transport close. Returns an unsubscribe fn. */
    onClose(handler: (info: { code?: number; reason?: string }) => void): () => void;
    /** Register a handler for transport-level errors. Returns an unsubscribe fn. */
    onError(handler: (err: unknown) => void): () => void;
}

/** The subset of the standard `WebSocket` interface the default transport needs. */
export interface WebSocketLike {
    readonly readyState: number;
    send(data: string): void;
    close(code?: number, reason?: string): void;
    addEventListener(type: 'open', listener: () => void): void;
    addEventListener(type: 'close', listener: (ev: { code?: number; reason?: string }) => void): void;
    addEventListener(type: 'error', listener: (ev: unknown) => void): void;
    addEventListener(type: 'message', listener: (ev: { data: unknown }) => void): void;
}

export type WebSocketFactory = (url: string) => WebSocketLike;

const WS_CONNECTING = 0;
const WS_OPEN = 1;
const WS_CLOSING = 2;

/**
 * Default transport backed by a `WebSocket`-like object. By default it uses the
 * global `WebSocket`; pass a `factory` to inject one (e.g. the `ws` package on
 * Node, or a mock in tests).
 */
export class WebSocketTransport implements Transport {
    private socket: WebSocketLike | null = null;
    private readonly url: string;
    private readonly factory: WebSocketFactory;
    private readonly messageHandlers = new Set<(data: string) => void>();
    private readonly closeHandlers = new Set<(info: { code?: number; reason?: string }) => void>();
    private readonly errorHandlers = new Set<(err: unknown) => void>();

    constructor(url: string, factory?: WebSocketFactory) {
        this.url = url;
        if (factory) {
            this.factory = factory;
        } else {
            const G = globalThis as { WebSocket?: new (url: string) => WebSocketLike };
            if (!G.WebSocket) {
                throw new Error('No global WebSocket available; pass a WebSocketFactory to WebSocketTransport.');
            }
            const Ctor = G.WebSocket;
            this.factory = (u) => new Ctor(u);
        }
    }

    get state(): TransportState {
        if (!this.socket) return 'closed';
        switch (this.socket.readyState) {
            case WS_CONNECTING:
                return 'connecting';
            case WS_OPEN:
                return 'open';
            case WS_CLOSING:
                return 'closing';
            default:
                return 'closed';
        }
    }

    connect(): Promise<void> {
        if (this.socket && this.socket.readyState === WS_OPEN) return Promise.resolve();

        return new Promise<void>((resolve, reject) => {
            const socket = this.factory(this.url);
            this.socket = socket;

            socket.addEventListener('open', () => resolve());
            socket.addEventListener('error', (ev: unknown) => {
                for (const h of this.errorHandlers) h(ev);
                if (this.state !== 'open') reject(ev instanceof Error ? ev : new Error('WebSocket connection error'));
            });
            socket.addEventListener('close', (ev: { code?: number; reason?: string }) => {
                for (const h of this.closeHandlers) h({ code: ev.code, reason: ev.reason });
            });
            socket.addEventListener('message', (ev: { data: unknown }) => {
                const data = typeof ev.data === 'string' ? ev.data : String(ev.data);
                for (const h of this.messageHandlers) h(data);
            });
        });
    }

    send(data: string): void {
        if (!this.socket || this.socket.readyState !== WS_OPEN) {
            throw new Error(`Cannot send: transport is "${this.state}"`);
        }
        this.socket.send(data);
    }

    close(code?: number, reason?: string): void {
        this.socket?.close(code, reason);
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
}
