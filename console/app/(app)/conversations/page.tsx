import Link from 'next/link';
import { getAdminClient } from '@/lib/session';
import { EmptyState, ErrorState, PageHeader } from '@/components/States';

export const dynamic = 'force-dynamic';

const PAGE_SIZE = 25;

/** Paged conversation list. `?cursor=` drives offset paging via `nextCursor`. */
export default async function ConversationsPage({ searchParams }: { searchParams: Promise<{ cursor?: string }> }) {
    const { cursor } = await searchParams;
    const offset = cursor ? Number(cursor) : 0;

    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let data;
    try {
        data = await client.conversations({ limit: PAGE_SIZE, cursor: offset });
    } catch (err) {
        return (
            <div>
                <PageHeader title="Conversations" />
                <ErrorState error={err} />
            </div>
        );
    }

    return (
        <div>
            <PageHeader title="Conversations" subtitle="Org-scoped chat history. Click a row to view its messages." />

            {data.conversations.length === 0 ? (
                <EmptyState message="No conversations yet." />
            ) : (
                <div className="card overflow-hidden p-0">
                    <table className="w-full">
                        <thead className="border-b border-ink-800 bg-ink-900">
                            <tr>
                                <th className="th">Name</th>
                                <th className="th">Platform</th>
                                <th className="th">Created</th>
                                <th className="th">Updated</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y divide-ink-800">
                            {data.conversations.map((c) => (
                                <tr key={c.id} className="transition hover:bg-ink-800">
                                    <td className="td">
                                        <Link href={`/conversations/${encodeURIComponent(c.id)}`} className="font-medium text-white hover:text-accent-soft">
                                            {c.name || c.id}
                                        </Link>
                                    </td>
                                    <td className="td">
                                        <span className="badge bg-slate-500/15 text-slate-300">{c.platform}</span>
                                    </td>
                                    <td className="td text-slate-500">{new Date(c.createdAt).toLocaleString()}</td>
                                    <td className="td text-slate-500">{new Date(c.updatedAt).toLocaleString()}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}

            <div className="mt-4 flex items-center justify-between">
                <div className="text-xs text-slate-500">
                    Showing {data.conversations.length} starting at #{offset}
                </div>
                <div className="flex gap-2">
                    {offset > 0 ? (
                        <Link href={`/conversations?cursor=${Math.max(0, offset - PAGE_SIZE)}`} className="btn-ghost">
                            ← Previous
                        </Link>
                    ) : null}
                    {data.nextCursor != null ? (
                        <Link href={`/conversations?cursor=${data.nextCursor}`} className="btn-ghost">
                            Next →
                        </Link>
                    ) : null}
                </div>
            </div>
        </div>
    );
}
