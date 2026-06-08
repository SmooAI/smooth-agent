import { getConfig } from '@/lib/config';
import { getAdminClient } from '@/lib/session';
import { PageHeader } from '@/components/States';

export const dynamic = 'force-dynamic';

/**
 * Settings (stub — read-only). Shows the configured model / gateway / auth-mode
 * and a backend liveness probe. CRUD (connector + settings write) lands in
 * increment 3, which needs new write endpoints on the admin API.
 */
export default async function SettingsPage() {
    const cfg = getConfig();

    let health = 'unknown';
    try {
        const client = await getAdminClient();
        if (client) {
            const h = await client.health();
            health = h.status;
        }
    } catch {
        health = 'unreachable';
    }

    const rows: Array<{ label: string; value: string; mono?: boolean }> = [
        { label: 'Console auth mode', value: cfg.authMode },
        { label: 'Backend auth mode', value: cfg.backendAuthMode },
        { label: 'Admin API base URL', value: cfg.adminBaseUrl, mono: true },
        { label: 'Model', value: cfg.model, mono: true },
        { label: 'Gateway URL', value: cfg.gatewayUrl, mono: true },
        { label: 'Backend health', value: health },
    ];

    return (
        <div>
            <PageHeader title="Settings" subtitle="Read-only configuration. Editing arrives in increment 3 (write endpoints)." />

            <div className="card max-w-2xl divide-y divide-ink-800 p-0">
                {rows.map((r) => (
                    <div key={r.label} className="flex items-center justify-between px-5 py-4">
                        <span className="text-sm text-slate-400">{r.label}</span>
                        <span className={`text-sm text-white ${r.mono ? 'font-mono text-xs' : ''}`}>{r.value}</span>
                    </div>
                ))}
            </div>

            <div className="card mt-6 max-w-2xl border-amber-500/30 bg-amber-500/5">
                <div className="text-sm font-semibold text-amber-300">Coming in increment 3</div>
                <ul className="mt-2 list-inside list-disc space-y-1 text-sm text-slate-400">
                    <li>Connector configuration (add / edit / trigger re-index)</li>
                    <li>Settings write (model, gateway, auth) via new admin write endpoints</li>
                    <li>Document-set management (rename, delete, re-tag)</li>
                </ul>
            </div>
        </div>
    );
}
