import Image from 'next/image';
import { redirect } from 'next/navigation';
import { getConfig } from '@/lib/config';
import { getSession } from '@/lib/session';
import { devLogin } from './actions';

/**
 * The login page. Renders one of two forms depending on the configured auth
 * mode:
 *
 * - **dev**: a username field that mints a local session and proceeds as Admin
 *   against an `AUTH_MODE=none` server. Only reachable when `CONSOLE_AUTH=dev`.
 * - **openauth**: a single "Continue with OpenAuth" button that kicks off the
 *   OAuth code flow against the configured issuer.
 */
export default async function LoginPage() {
    const cfg = getConfig();
    const session = await getSession();
    if (session) redirect('/');

    return (
        <div className="flex min-h-screen items-center justify-center bg-ink-950 px-4">
            <div className="w-full max-w-sm">
                <div className="mb-8 flex flex-col items-center text-center">
                    <Image src="/smooth-logo.svg" alt="Smooth" width={56} height={56} priority />
                    <h1 className="mt-4 text-xl font-semibold text-white">Smooth Operator</h1>
                    <p className="text-sm text-slate-500">Management Console</p>
                </div>

                <div className="card">
                    {cfg.authMode === 'dev' ? (
                        <form action={devLogin} className="space-y-4">
                            <div>
                                <label className="card-title" htmlFor="username">
                                    Dev sign-in
                                </label>
                                <input
                                    id="username"
                                    name="username"
                                    defaultValue="dev-admin"
                                    className="mt-2 w-full rounded-lg border border-ink-600 bg-ink-800 px-3 py-2 text-sm text-white outline-none focus:border-accent"
                                    placeholder="username"
                                />
                            </div>
                            <p className="text-xs text-slate-500">
                                Dev mode — proceeds as <span className="text-accent-soft">Admin</span> against a server running{' '}
                                <code className="text-slate-400">AUTH_MODE=none</code>. No real token is used.
                            </p>
                            <button type="submit" className="btn w-full">
                                Continue as Admin
                            </button>
                        </form>
                    ) : (
                        <form action="/api/auth/login" method="get" className="space-y-4">
                            <p className="text-sm text-slate-400">
                                Sign in with your organization&apos;s identity provider (SST OpenAuth / Smoo identity).
                            </p>
                            <button type="submit" className="btn w-full">
                                Continue with OpenAuth
                            </button>
                        </form>
                    )}
                </div>

                <p className="mt-4 text-center text-xs text-slate-600">
                    Auth mode: <span className="text-slate-400">{cfg.authMode}</span>
                </p>
            </div>
        </div>
    );
}
