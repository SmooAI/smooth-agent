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

/** Default connect timeout (ms) for the WebSocket transport. */
const DEFAULT_CONNECT_TIMEOUT = 30_000;

/**
 * Default transport backed by a `WebSocket`-like object. By default it uses the
 * global `WebSocket`; pass a `factory` to inject one (e.g. the `ws` package on
 * Node, or a mock in tests).
 */
export class WebSocketTransport implements Transport {
    private socket: WebSocketLike | null = null;
    private readonly url: string;
    private readonly factory: WebSocketFactory;
    private readonly connectTimeout: number;
    private readonly messageHandlers = new Set<(data: string) => void>();
    private readonly closeHandlers = new Set<(info: { code?: number; reason?: string }) => void>();
    private readonly errorHandlers = new Set<(err: unknown) => void>();

    constructor(url: string, factory?: WebSocketFactory, connectTimeout = DEFAULT_CONNECT_TIMEOUT) {
        this.url = url;
        this.connectTimeout = connectTimeout;
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

        // A prior socket that never reached OPEN (failed/half-open dial, or a closed
        // socket from a previous attempt) would otherwise be orphaned: its listeners
        // stay registered and keep dispatching into the shared handler sets, so a late
        // message/close from the dead socket double-fires. Close it and detach it
        // before dialing a fresh one. (WebSocketLike has no removeEventListener, so we
        // also guard every handler below with an identity check on `this.socket`.)
        if (this.socket && this.socket.readyState !== WS_OPEN) {
            const stale = this.socket;
            this.socket = null;
            try {
                stale.close();
            } catch {
                // ignore — best-effort teardown of a half-open socket
            }
        }

        return new Promise<void>((resolve, reject) => {
            const socket = this.factory(this.url);
            this.socket = socket;

            let settled = false;
            const timer =
                this.connectTimeout > 0
                    ? setTimeout(() => {
                          if (settled) return;
                          settled = true;
                          // Tear down the half-open socket so it can't leak / fire later.
                          if (this.socket === socket) this.socket = null;
                          try {
                              socket.close();
                          } catch {
                              // ignore
                          }
                          reject(new Error(`WebSocket connect to ${this.url} timed out after ${this.connectTimeout}ms`));
                      }, this.connectTimeout)
                    : undefined;

            socket.addEventListener('open', () => {
                // Ignore events from a socket we've already replaced/abandoned.
                if (this.socket !== socket) return;
                if (settled) return;
                settled = true;
                if (timer) clearTimeout(timer);
                resolve();
            });
            socket.addEventListener('error', (ev: unknown) => {
                if (this.socket !== socket) return;
                for (const h of this.errorHandlers) h(ev);
                if (!settled && this.state !== 'open') {
                    settled = true;
                    if (timer) clearTimeout(timer);
                    if (this.socket === socket) this.socket = null;
                    // Release the failed socket so it can't linger / fire later.
                    try {
                        socket.close();
                    } catch {
                        // ignore
                    }
                    reject(ev instanceof Error ? ev : new Error('WebSocket connection error'));
                }
            });
            socket.addEventListener('close', (ev: { code?: number; reason?: string }) => {
                if (this.socket !== socket) return;
                if (timer) clearTimeout(timer);
                for (const h of this.closeHandlers) h({ code: ev.code, reason: ev.reason });
            });
            socket.addEventListener('message', (ev: { data: unknown }) => {
                if (this.socket !== socket) return;
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
