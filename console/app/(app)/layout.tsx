import { redirect } from 'next/navigation';
import { Header } from '@/components/Header';
import { Sidebar } from '@/components/Sidebar';
import { getSession } from '@/lib/session';
import { getAdminClient } from '@/lib/session';
import type { Principal } from '@/lib/types';

/**
 * The authenticated app shell. Redirects unauthenticated requests to `/login`,
 * then resolves the live principal via `/admin/me` (falling back to the session
 * hint) so the header + role-aware sidebar always reflect the real role.
 */
export default async function AppLayout({ children }: { children: React.ReactNode }) {
    const session = await getSession();
    if (!session) redirect('/login');

    // Prefer the live principal; fall back to the cached session hint so the
    // shell still renders if /admin/me is briefly unreachable.
    let principal: Principal | undefined = session.principal;
    try {
        const client = await getAdminClient();
        if (client) principal = await client.me();
    } catch {
        // keep the session hint
    }

    return (
        <div className="flex min-h-screen flex-col">
            <Header principal={principal} />
            <div className="flex flex-1">
                <Sidebar role={principal?.role} />
                <main className="flex-1 overflow-auto p-8">{children}</main>
            </div>
        </div>
    );
}
