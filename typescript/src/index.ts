/**
 * @smooai/smooth-agent — TypeScript protocol types + native client for the
 * smooth-agent WebSocket protocol.
 *
 * The protocol contract is defined by the language-neutral JSON Schemas in
 * `spec/`. The generated types (`./generated/types.ts`) are committed so consumers
 * don't need the generator; `./types.ts` layers the ergonomic discriminated
 * unions and guards on top.
 */
export * from './types.js';
export {
    SmoothAgentClient,
    MessageTurn,
    ProtocolError,
    type SmoothAgentClientOptions,
} from './client.js';
export {
    WebSocketTransport,
    type Transport,
    type TransportState,
    type WebSocketLike,
    type WebSocketFactory,
} from './transport.js';
export {
    ProtocolValidator,
    DEFAULT_SPEC_DIR,
    formatErrors,
    type ValidationResult,
} from './validate.js';
