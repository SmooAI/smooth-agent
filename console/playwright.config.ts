import { defineConfig } from '@playwright/test';

/**
 * Playwright config for the console smoke test.
 *
 * The test is gated on `SMOOTH_AGENT_E2E=1` (like the other live tests in this
 * repo) and self-manages booting the `smooth-operator-server` (AUTH_MODE=none)
 * plus the Next.js console. We use the `list` reporter (never the HTML reporter
 * with `open: 'auto'`, which spawns a blocking server).
 */
export default defineConfig({
    testDir: './e2e',
    timeout: 120_000,
    fullyParallel: false,
    workers: 1,
    reporter: 'list',
    use: {
        baseURL: process.env.CONSOLE_URL ?? 'http://127.0.0.1:3939',
        trace: 'retain-on-failure',
    },
    projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
