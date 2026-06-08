import { NextRequest, NextResponse } from 'next/server';
import { randomBytes } from 'node:crypto';
import { getConfig } from '@/lib/config';
import { buildAuthorizeUrl } from '@/lib/openauth';

/**
 * `GET /api/auth/login` — kick off the OpenAuth (BYO) authorization-code flow.
 *
 * Redirects the browser to the issuer's `/authorize`, stashing a CSRF `state`
 * value in a short-lived cookie that the callback validates. Dev mode never
 * reaches here (its login is a server action).
 */
export async function GET(req: NextRequest) {
    const cfg = getConfig();
    if (cfg.authMode === 'dev') {
        return NextResponse.redirect(new URL('/login', req.url));
    }
    if (!cfg.openauthIssuer) {
        return NextResponse.json({ error: 'OPENAUTH_ISSUER is not configured' }, { status: 500 });
    }

    const state = randomBytes(16).toString('hex');
    const redirectUri = new URL('/api/auth/callback', req.url).toString();
    const authorizeUrl = buildAuthorizeUrl(redirectUri, state);

    const res = NextResponse.redirect(authorizeUrl);
    res.cookies.set('smooth_oauth_state', state, {
        httpOnly: true,
        sameSite: 'lax',
        secure: process.env.NODE_ENV === 'production',
        path: '/',
        maxAge: 600,
    });
    return res;
}
