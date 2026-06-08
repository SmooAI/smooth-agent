import { SettingsForm } from '@/components/SettingsForm';
import { ErrorState, PageHeader } from '@/components/States';
import { getConfig } from '@/lib/config';
import { canManage } from '@/lib/rbac';
import { getAdminClient } from '@/lib/session';
import type { AgentSettings, Role } from '@/lib/types';

export const dynamic = 'force-dynamic';

/**
 * Settings — an editable agent-settings form (model / system prompt / default
 * tools; Admin writes via `PUT /admin/settings`, Curator views) plus a
 * read-only block of backend health + deploy config.
 */
export default async function SettingsPage() {
    const cfg = getConfig();
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let settings: AgentSettings | null = null;
    let role: Role | undefined;
    let settingsError: unknown = null;
    let health = 'unknown';

    try {
        const [s, p, h] = await Promise.allSettled([client.getSettings(), client.me(), client.health()]);
        if (s.status === 'fulfilled') settings = s.value;
        else settingsError = s.reason;
        if (p.status === 'fulfilled') role = p.value.role;
        health = h.status === 'fulfilled' ? h.value.status : 'unreachable';
    } catch (err) {
        settingsError = err;
    }

    const manage = canManage(role);

    const rows: Array<{ label: string; value: string; mono?: boolean }> = [
        { label: 'Console auth mode', value: cfg.authMode },
        { label: 'Backend auth mode', value: cfg.backendAuthMode },
        { label: 'Admin API base URL', value: cfg.adminBaseUrl, mono: true },
        { label: 'Deploy model (env)', value: cfg.model, mono: true },
        { label: 'Gateway URL', value: cfg.gatewayUrl, mono: true },
        { label: 'Backend health', value: health },
    ];

    return (
        <div>
            <PageHeader title="Settings" subtitle="Agent configuration (editable by Admin) and read-only backend/deploy config." />

            <h2 className="mb-3 text-sm font-semibold text-slate-300">Agent settings</h2>
            {settings ? (
                <SettingsForm settings={settings} canManage={manage} />
            ) : (
                <ErrorState error={settingsError ?? 'Settings unavailable.'} />
            )}

            <h2 className="mb-3 mt-8 text-sm font-semibold text-slate-300">Backend &amp; deploy</h2>
            <div className="card max-w-2xl divide-y divide-ink-800 p-0">
                {rows.map((r) => (
                    <div key={r.label} className="flex items-center justify-between px-5 py-4">
                        <span className="text-sm text-slate-400">{r.label}</span>
                        <span className={`text-sm text-white ${r.mono ? 'font-mono text-xs' : ''}`}>{r.value}</span>
                    </div>
                ))}
            </div>
        </div>
    );
}
