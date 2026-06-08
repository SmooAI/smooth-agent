import { getAdminClient } from '@/lib/session';
import { StatusBadge } from '@/components/Badges';
import { EmptyState, ErrorState, PageHeader } from '@/components/States';

export const dynamic = 'force-dynamic';

/** Indexing runs table (Curator+). Surfaces a 403 as a permissions error. */
export default async function IndexingPage() {
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let data;
    try {
        data = await client.indexingRuns();
    } catch (err) {
        return (
            <div>
                <PageHeader title="Indexing" />
                <ErrorState error={err} />
            </div>
        );
    }

    return (
        <div>
            <PageHeader title="Indexing" subtitle="Indexing-run status across the organization's connectors." />

            {data.runs.length === 0 ? (
                <EmptyState message="No indexing runs recorded yet." />
            ) : (
                <div className="card overflow-hidden p-0">
                    <table className="w-full">
                        <thead className="border-b border-ink-800 bg-ink-900">
                            <tr>
                                <th className="th">Connector</th>
                                <th className="th">Status</th>
                                <th className="th">Seen</th>
                                <th className="th">Indexed</th>
                                <th className="th">Skipped</th>
                                <th className="th">Started</th>
                                <th className="th">Finished</th>
                                <th className="th">Cursor</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y divide-ink-800">
                            {data.runs.map((r) => (
                                <tr key={r.id} className="transition hover:bg-ink-800">
                                    <td className="td font-medium text-white">{r.connectorName}</td>
                                    <td className="td">
                                        <StatusBadge status={r.status} />
                                        {r.error ? <div className="mt-1 text-xs text-rose-400">{r.error}</div> : null}
                                    </td>
                                    <td className="td">{r.documentsSeen}</td>
                                    <td className="td">{r.chunksIndexed}</td>
                                    <td className="td">{r.documentsSkipped}</td>
                                    <td className="td text-slate-500">{new Date(r.startedAt).toLocaleString()}</td>
                                    <td className="td text-slate-500">{r.finishedAt ? new Date(r.finishedAt).toLocaleString() : '—'}</td>
                                    <td className="td font-mono text-xs text-slate-500">{r.cursor ?? '—'}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    );
}
