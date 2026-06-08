import { NextRequest, NextResponse } from 'next/server';
import { getConfig } from '@/lib/config';
import { exchangeCode, decodePrincipalHint } from '@/lib/openauth';

/**
 * `GET /api/auth/callback` — the OpenAuth redirect target.
 *
 * Validates the CSRF `state`, exchanges the `code` for the OpenAuth-issued JWT
 * (carrying `sub` / `org` / `role`), and stores it as the session bearer. The
 * admin API re-verifies that JWT on every request, so the console never trusts
 * the token itself — it only forwards it.
 */
export async function GET(req: NextRequest) {
    const cfg = getConfig();
    if (cfg.authMode === 'dev') {
        return NextResponse.redirect(new URL('/login', req.url));
    }

    const url = new URL(req.url);
    const code = url.searchParams.get('code');
    const state = url.searchParams.get('state');
    const expectedState = req.cookies.get('smooth_oauth_state')?.value;

    if (!code || !state || !expectedState || state !== expectedState) {
        return NextResponse.redirect(new URL('/login?error=oauth_state', req.url));
    }

    const redirectUri = new URL('/api/auth/callback', req.url).toString();
    let jwt: string;
    try {
        jwt = await exchangeCode(code, redirectUri);
    } catch {
        return NextResponse.redirect(new URL('/login?error=exchange', req.url));
    }

    const principal = decodePrincipalHint(jwt);
    const session = JSON.stringify({ token: jwt, principal });

    const res = NextResponse.redirect(new URL('/', req.url));
    res.cookies.set('smooth_console_session', session, {
        httpOnly: true,
        sameSite: 'lax',
        secure: process.env.NODE_ENV === 'production',
        path: '/',
        maxAge: 60 * 60 * 8,
    });
    res.cookies.delete('smooth_oauth_state');
    return res;
}
