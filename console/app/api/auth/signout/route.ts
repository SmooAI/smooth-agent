import { NextRequest, NextResponse } from 'next/server';

/** `POST /api/auth/signout` — clear the session and bounce to login. */
export async function POST(req: NextRequest) {
    const res = NextResponse.redirect(new URL('/login', req.url));
    res.cookies.delete('smooth_console_session');
    return res;
}
