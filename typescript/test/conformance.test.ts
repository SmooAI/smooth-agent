/**
 * Conformance: every instance in `spec/conformance/fixtures.json` must validate
 * against the schema it claims to (mirrors the spec's own ajv-cli validation, in TS).
 */
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';
import { ProtocolValidator, formatErrors } from '../src/validate.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SPEC_DIR = join(__dirname, '..', '..', 'spec');

interface Fixture {
    $schema_ref: string;
    description: string;
    instance: unknown;
}

let validator: ProtocolValidator;
let fixtures: Record<string, Fixture>;

beforeAll(async () => {
    validator = await ProtocolValidator.load(SPEC_DIR);
    const raw = JSON.parse(await readFile(join(SPEC_DIR, 'conformance', 'fixtures.json'), 'utf8')) as Record<
        string,
        unknown
    >;
    fixtures = Object.fromEntries(Object.entries(raw).filter(([k]) => !k.startsWith('$'))) as Record<string, Fixture>;
});

describe('conformance fixtures', () => {
    it('exposes at least the five documented fixtures', () => {
        expect(Object.keys(fixtures)).toEqual(
            expect.arrayContaining([
                'create_session_request',
                'create_session_response',
                'send_message_request',
                'stream_chunk_event',
                'eventual_response_event',
            ]),
        );
    });

    it('validates every fixture against its declared schema ref', async () => {
        for (const [name, fixture] of Object.entries(fixtures)) {
            const result = validator.validateAt(fixture.$schema_ref, fixture.instance);
            expect(result.valid, `${name} (${fixture.$schema_ref}): ${formatErrors(result.errors)}`).toBe(true);
        }
    });

    it('rejects a fixture mutated to violate its schema', () => {
        const fixture = fixtures.stream_chunk_event!;
        const broken = structuredClone(fixture.instance) as { type: string };
        broken.type = 'not_a_real_event';
        const result = validator.validateAt(fixture.$schema_ref, broken);
        expect(result.valid).toBe(false);
        expect(result.errors.length).toBeGreaterThan(0);
    });
});

describe('discriminator-based validation', () => {
    it('validateAction routes a send_message request to its schema', async () => {
        const v = validator;
        const send = fixtures.send_message_request!.instance as { action: 'send_message' } & Record<string, unknown>;
        expect(v.validateAction(send).valid).toBe(true);
    });

    it('validateEvent routes a stream_chunk event to its schema', () => {
        const chunk = fixtures.stream_chunk_event!.instance as { type: 'stream_chunk' } & Record<string, unknown>;
        expect(validator.validateEvent(chunk).valid).toBe(true);
    });

    it('validateAction rejects a malformed action (missing required field)', () => {
        const result = validator.validateAction({ action: 'send_message', sessionId: 'x' } as never);
        expect(result.valid).toBe(false);
    });
});
