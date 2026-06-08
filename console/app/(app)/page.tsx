import Link from 'next/link';
import { getAdminClient } from '@/lib/session';
import { canCurate } from '@/lib/rbac';
import { RoleBadge, StatusBadge } from '@/components/Badges';
import { ErrorState, PageHeader } from '@/components/States';
import type { Principal } from '@/lib/types';

export const dynamic = 'force-dynamic';

/** Overview dashboard — summary cards across the read API. */
export default async function DashboardPage() {
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let principal: Principal;
    try {
        principal = await client.me();
    } catch (err) {
        return <ErrorState error={err} />;
    }

    const curator = canCurate(principal.role);

    // Fetch the summary data in parallel; each card degrades independently.
    const [convResult, docSetsResult, runsResult] = await Promise.allSettled([
        client.conversations({ limit: 200 }),
        curator ? client.documentSets() : Promise.resolve(null),
        curator ? client.indexingRuns() : Promise.resolve(null),
    ]);

    const convCount = convResult.status === 'fulfilled' ? convResult.value.conversations.length : null;
    const docSets = docSetsResult.status === 'fulfilled' ? docSetsResult.value : null;
    const runs = runsResult.status === 'fulfilled' ? runsResult.value : null;

    const docSetCount = docSets?.documentSets.length ?? null;
    const docCount = docSets?.documentSets.reduce((n, s) => n + s.documentCount, 0) ?? null;
    const runningRuns = runs?.runs.filter((r) => r.status === 'running').length ?? 0;
    const failedRuns = runs?.runs.filter((r) => r.status === 'failed').length ?? 0;

    return (
        <div>
            <PageHeader title="Dashboard" subtitle="A live overview of conversations, indexing, and knowledge." />

            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <Link href="/conversations" className="card transition hover:border-accent/40">
                    <div className="card-title">Conversations</div>
                    <div className="stat">{convCount ?? '—'}</div>
                    <p className="mt-1 text-xs text-slate-500">Org-scoped chat history</p>
                </Link>

                <div className="card">
                    <div className="card-title">Signed in as</div>
                    <div className="stat truncate text-xl">{principal.displayName ?? principal.userId}</div>
                    <div className="mt-2 flex items-center gap-2">
                        <RoleBadge role={principal.role} />
                        <span className="text-xs text-slate-500">{principal.orgId}</span>
                    </div>
                </div>

                {curator ? (
                    <>
                        <Link href="/document-sets" className="card transition hover:border-accent/40">
                            <div className="card-title">Document Sets</div>
                            <div className="stat">{docSetCount ?? '—'}</div>
                            <p className="mt-1 text-xs text-slate-500">{docCount ?? '—'} documents</p>
                        </Link>

                        <Link href="/indexing" className="card transition hover:border-accent/40">
                            <div className="card-title">Indexing Runs</div>
                            <div className="stat">{runs?.runs.length ?? '—'}</div>
                            <p className="mt-1 text-xs text-slate-500">
                                {runningRuns} running · {failedRuns} failed
                            </p>
                        </Link>
                    </>
                ) : (
                    <div className="card sm:col-span-2">
                        <div className="card-title">Knowledge</div>
                        <p className="mt-3 text-sm text-slate-400">
                            Indexing and document-set details are available to Curator and Admin roles.
                        </p>
                    </div>
                )}
            </div>

            {curator && runs && runs.runs.length > 0 ? (
                <div className="mt-8">
                    <h2 className="mb-3 text-sm font-semibold text-slate-300">Recent indexing activity</h2>
                    <div className="card overflow-hidden p-0">
                        <table className="w-full">
                            <tbody className="divide-y divide-ink-800">
                                {runs.runs.slice(0, 5).map((r) => (
                                    <tr key={r.id}>
                                        <td className="td font-medium text-white">{r.connectorName}</td>
                                        <td className="td">
                                            <StatusBadge status={r.status} />
                                        </td>
                                        <td className="td text-slate-400">{r.chunksIndexed} chunks</td>
                                        <td className="td text-slate-500">{new Date(r.startedAt).toLocaleString()}</td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                </div>
            ) : null}
        </div>
    );
}
