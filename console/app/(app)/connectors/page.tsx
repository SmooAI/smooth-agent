import Link from 'next/link';
import { ConnectorRowActions } from '@/components/ConnectorRowActions';
import { EmptyState, ErrorState, PageHeader } from '@/components/States';
import { canManage } from '@/lib/rbac';
import { getAdminClient } from '@/lib/session';
import type { ConnectorConfig } from '@/lib/types';

export const dynamic = 'force-dynamic';

/** Short, human label for a connector's source from its kind-specific config. */
function source(c: ConnectorConfig): string {
    const cfg = c.config;
    if (c.kind === 'github') {
        const owner = typeof cfg.owner === 'string' ? cfg.owner : '?';
        const repo = typeof cfg.repo === 'string' ? cfg.repo : '?';
        return `${owner}/${repo}`;
    }
    if (c.kind === 'web') return typeof cfg.url === 'string' ? cfg.url : '—';
    if (c.kind === 'file') return typeof cfg.path === 'string' ? cfg.path : '—';
    return '—';
}

const KIND_STYLE: Record<string, string> = {
    github: 'bg-violet-500/15 text-violet-300',
    web: 'bg-sky-500/15 text-sky-300',
    file: 'bg-emerald-500/15 text-emerald-300',
};

/**
 * Connectors list (Curator+ to view; Admin to create/edit/delete). Each row has
 * an "Index now" action (Curator+) plus Edit/Delete (Admin). A 403 surfaces as a
 * friendly permissions error.
 */
export default async function ConnectorsPage() {
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let data;
    let role;
    try {
        [data, role] = await Promise.all([client.listConnectors(), client.me().then((p) => p.role)]);
    } catch (err) {
        return (
            <div>
                <PageHeader title="Connectors" />
                <ErrorState error={err} />
            </div>
        );
    }

    const manage = canManage(role);

    return (
        <div>
            <div className="mb-6 flex items-start justify-between">
                <PageHeader title="Connectors" subtitle="GitHub / web / file sources the indexing loop pulls from. Index on demand." />
                {manage ? (
                    <Link href="/connectors/new" className="btn" data-testid="new-connector">
                        + New connector
                    </Link>
                ) : null}
            </div>

            {data.connectors.length === 0 ? (
                <EmptyState message={manage ? 'No connectors yet. Add one to start indexing a source.' : 'No connectors configured.'} />
            ) : (
                <div className="card overflow-hidden p-0">
                    <table className="w-full">
                        <thead className="border-b border-ink-800 bg-ink-900">
                            <tr>
                                <th className="th">Name</th>
                                <th className="th">Kind</th>
                                <th className="th">Source</th>
                                <th className="th">Enabled</th>
                                <th className="th">Updated</th>
                                <th className="th text-right">Actions</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y divide-ink-800">
                            {data.connectors.map((c) => (
                                <tr key={c.id} className="transition hover:bg-ink-800" data-testid="connector-row">
                                    <td className="td font-medium text-white">{c.name}</td>
                                    <td className="td">
                                        <span className={`badge ${KIND_STYLE[c.kind] ?? 'bg-slate-500/15 text-slate-300'}`}>{c.kind}</span>
                                    </td>
                                    <td className="td max-w-xs truncate font-mono text-xs text-slate-400" title={source(c)}>
                                        {source(c)}
                                    </td>
                                    <td className="td">
                                        {c.enabled ? (
                                            <span className="badge bg-emerald-500/15 text-emerald-300">enabled</span>
                                        ) : (
                                            <span className="badge bg-slate-500/15 text-slate-400">disabled</span>
                                        )}
                                    </td>
                                    <td className="td text-slate-500">{new Date(c.updatedAt).toLocaleString()}</td>
                                    <td className="td">
                                        <ConnectorRowActions id={c.id} name={c.name} canManage={manage} />
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    );
}
