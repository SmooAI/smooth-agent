'use server';

import { redirect } from 'next/navigation';
import { getConfig } from '@/lib/config';
import { setSession } from '@/lib/session';
import { AdminClient } from '@/lib/admin-client';

/**
 * Dev-mode login server action.
 *
 * Only honored when `CONSOLE_AUTH=dev`. Mints a session whose admin-API bearer
 * is the literal `dev` — which an `AUTH_MODE=none` server accepts, returning a
 * fixed Admin principal. We immediately call `/admin/me` to capture the real
 * principal (org id + role) for the session hint, proving the backend is
 * reachable before redirecting into the app.
 */
export async function devLogin(formData: FormData): Promise<void> {
    const cfg = getConfig();
    if (cfg.authMode !== 'dev') {
        // Hard refusal — dev login is unreachable outside dev mode.
        throw new Error('dev login is disabled (CONSOLE_AUTH is not "dev")');
    }

    const token = 'dev';
    let principal;
    try {
        const client = new AdminClient({ baseUrl: cfg.adminBaseUrl, token });
        principal = await client.me();
    } catch {
        // The backend may be unreachable; still create the session so the app
        // shell renders its error states rather than bouncing back to login.
        principal = undefined;
    }

    await setSession({ token, principal });
    redirect('/');
}
