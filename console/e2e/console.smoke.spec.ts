/**
 * Console smoke test (live).
 *
 * Gated on `SMOOTH_AGENT_E2E=1`. Boots a `smooth-operator-server` with
 * `AUTH_MODE=none` (a fixed Admin principal — no real token needed) +
 * `SMOOTH_AGENT_SEED_KB=1` (seeds the `policies` document set), then boots the
 * Next.js console in dev-auth mode pointed at it, signs in via the dev login,
 * and asserts the dashboard + document-sets pages render data fetched from the
 * admin API.
 *
 * Run:
 *   SMOOTH_AGENT_E2E=1 pnpm test:e2e
 *
 * Skips cleanly (whole file) when `SMOOTH_AGENT_E2E` is unset. Never prints
 * secrets — the gateway key, if present, is passed via env, never logged.
 */
import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath } from 'node:url';

const E2E = process.env.SMOOTH_AGENT_E2E === '1';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SERVER_BIN = path.join(process.env.HOME ?? '', '.cargo/shared-target/debug/smooth-operator-server');
const CONSOLE_DIR = path.resolve(HERE, '..');

const ADMIN_PORT = 8840;
const CONSOLE_PORT = 3939;
const ADMIN_URL = `http://127.0.0.1:${ADMIN_PORT}`;
const CONSOLE_URL = `http://127.0.0.1:${CONSOLE_PORT}`;

let serverProc: ChildProcess | undefined;
let consoleProc: ChildProcess | undefined;

/** Poll a TCP port until something is listening (or timeout). */
async function waitForPort(port: number, timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const ok = await new Promise<boolean>((resolve) => {
            const sock = net.connect(port, '127.0.0.1');
            sock.once('connect', () => {
                sock.destroy();
                resolve(true);
            });
            sock.once('error', () => resolve(false));
        });
        if (ok) return;
        await new Promise((r) => setTimeout(r, 300));
    }
    throw new Error(`port ${port} did not open within ${timeoutMs}ms`);
}

/** Poll an HTTP URL for a 2xx (the admin health probe). */
async function waitForHealth(url: string, timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const res = await fetch(url);
            if (res.ok) return;
        } catch {
            // not up yet
        }
        await new Promise((r) => setTimeout(r, 300));
    }
    throw new Error(`health ${url} not ready within ${timeoutMs}ms`);
}

test.describe('console smoke (live)', () => {
    test.skip(!E2E, 'set SMOOTH_AGENT_E2E=1 to run the live console smoke test');

    test.beforeAll(async () => {
        // 1. Boot smooth-operator-server with AUTH_MODE=none + seeded KB.
        serverProc = spawn(SERVER_BIN, [], {
            env: {
                ...process.env,
                AUTH_MODE: 'none',
                SMOOTH_AGENT_PORT: String(ADMIN_PORT),
                SMOOTH_AGENT_SEED_KB: '1',
                // SMOOTH_AGENT_GATEWAY_KEY (if set in env) flows through; not logged.
            },
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        serverProc.stderr?.on('data', (d) => process.stdout.write(`[server] ${d}`));
        await waitForHealth(`${ADMIN_URL}/admin/health`, 30_000);

        // 2. Boot the console (next start, dev-auth mode) pointed at the server.
        //    next start serves the already-built .next from `pnpm build`.
        consoleProc = spawn('node_modules/.bin/next', ['start', '-p', String(CONSOLE_PORT)], {
            cwd: CONSOLE_DIR,
            env: {
                ...process.env,
                CONSOLE_AUTH: 'dev',
                ADMIN_API_URL: ADMIN_URL,
                BACKEND_AUTH_MODE: 'none',
                PORT: String(CONSOLE_PORT),
            },
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        consoleProc.stdout?.on('data', (d) => process.stdout.write(`[console] ${d}`));
        consoleProc.stderr?.on('data', (d) => process.stdout.write(`[console] ${d}`));
        await waitForPort(CONSOLE_PORT, 30_000);
    });

    test.afterAll(async () => {
        for (const proc of [consoleProc, serverProc]) {
            if (proc && !proc.killed) {
                proc.kill('SIGTERM');
                await Promise.race([once(proc, 'exit'), new Promise((r) => setTimeout(r, 3000))]);
            }
        }
    });

    test('dev login → dashboard shows Admin role + conversations card', async ({ page }) => {
        page.on('console', (msg) => console.log(`[browser] ${msg.text()}`));

        // Land on /login (unauthenticated redirect), submit the dev login.
        await page.goto(`${CONSOLE_URL}/`);
        await page.waitForURL(/\/login/);
        await expect(page.getByText('Dev sign-in')).toBeVisible();
        await page.getByRole('button', { name: 'Continue as Admin' }).click();

        // Dashboard renders the principal from /admin/me — the AUTH_MODE=none
        // server returns a fixed Admin principal.
        await page.waitForURL(`${CONSOLE_URL}/`);
        await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible();
        // The Admin role badge (from the live /admin/me) renders in both the
        // header and the dashboard card — assert at least one is visible.
        await expect(page.getByText('admin', { exact: true }).first()).toBeVisible();
        await expect(page.getByText('Conversations').first()).toBeVisible();
    });

    test('document-sets page shows the seeded "policies" set', async ({ page }) => {
        // Reuse the session by signing in again (each test gets a fresh context).
        await page.goto(`${CONSOLE_URL}/login`);
        await page.getByRole('button', { name: 'Continue as Admin' }).click();
        await page.waitForURL(`${CONSOLE_URL}/`);

        await page.goto(`${CONSOLE_URL}/document-sets`);
        await expect(page.getByRole('heading', { name: 'Document Sets' })).toBeVisible();
        // The seeded server tags its demo docs into the `policies` document set.
        await expect(page.getByText('policies')).toBeVisible();
    });
});
