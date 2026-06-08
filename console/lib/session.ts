/**
 * Session handling for the console.
 *
 * A session is a small JSON blob stored in an httpOnly cookie holding the bearer
 * token used against the admin API plus a cached principal hint. In `dev` mode
 * the token is the literal `dev`; in `openauth` mode it is the OpenAuth-issued
 * JWT obtained at the auth callback.
 *
 * The cookie is the only place the admin-API bearer lives client-side — it is
 * never exposed to client components. Pages read the session in server
 * components and construct an {@link AdminClient} per request.
 */

import { cookies } from 'next/headers';
import { AdminClient } from './admin-client';
import { getConfig } from './config';
import type { Principal } from './types';

const COOKIE_NAME = 'smooth_console_session';

export interface Session {
    token: string;
    /** Cached principal from the login exchange / dev login (display hint). */
    principal?: Principal;
}

/** Read the session from the request cookies, or `null` when signed out. */
export async function getSession(): Promise<Session | null> {
    const store = await cookies();
    const raw = store.get(COOKIE_NAME)?.value;
    if (!raw) return null;
    try {
        return JSON.parse(raw) as Session;
    } catch {
        return null;
    }
}

/** Persist a session into an httpOnly cookie. */
export async function setSession(session: Session): Promise<void> {
    const store = await cookies();
    store.set(COOKIE_NAME, JSON.stringify(session), {
        httpOnly: true,
        sameSite: 'lax',
        secure: process.env.NODE_ENV === 'production',
        path: '/',
        maxAge: 60 * 60 * 8, // 8h
    });
}

/** Clear the session cookie (sign out). */
export async function clearSession(): Promise<void> {
    const store = await cookies();
    store.delete(COOKIE_NAME);
}

/** Build an {@link AdminClient} bound to the current session's bearer token. */
export async function getAdminClient(): Promise<AdminClient | null> {
    const session = await getSession();
    if (!session) return null;
    return new AdminClient({ baseUrl: getConfig().adminBaseUrl, token: session.token });
}
