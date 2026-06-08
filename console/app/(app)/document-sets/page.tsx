import { getAdminClient } from '@/lib/session';
import { EmptyState, ErrorState, PageHeader } from '@/components/States';

export const dynamic = 'force-dynamic';

/** Document sets list (Curator+) — distinct set names + document counts. */
export default async function DocumentSetsPage() {
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let data;
    try {
        data = await client.documentSets();
    } catch (err) {
        return (
            <div>
                <PageHeader title="Document Sets" />
                <ErrorState error={err} />
            </div>
        );
    }

    const total = data.documentSets.reduce((n, s) => n + s.documentCount, 0);

    return (
        <div>
            <PageHeader title="Document Sets" subtitle={`${data.documentSets.length} sets · ${total} documents`} />

            {data.documentSets.length === 0 ? (
                <EmptyState message="No document sets yet. Seed the knowledge base or run a connector." />
            ) : (
                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
                    {data.documentSets.map((s) => (
                        <div key={s.name} className="card" data-testid="document-set">
                            <div className="card-title">Document Set</div>
                            <div className="mt-2 text-lg font-semibold text-white">{s.name}</div>
                            <div className="mt-3 flex items-baseline gap-2">
                                <span className="text-2xl font-semibold text-accent-soft">{s.documentCount}</span>
                                <span className="text-xs text-slate-500">documents</span>
                            </div>
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}
