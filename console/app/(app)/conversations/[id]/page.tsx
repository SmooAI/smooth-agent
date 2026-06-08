import Link from 'next/link';
import { getAdminClient } from '@/lib/session';
import { EmptyState, ErrorState, PageHeader } from '@/components/States';
import type { Message } from '@/lib/types';

export const dynamic = 'force-dynamic';

/** Best-effort text extraction from a message's structured content. */
function messageText(m: Message): string {
    if (m.content.text) return m.content.text;
    const fromItems = m.content.items
        .map((i) => i.text)
        .filter(Boolean)
        .join('\n');
    if (fromItems) return fromItems;
    if (m.content.structuredResponse) return JSON.stringify(m.content.structuredResponse, null, 2);
    return '(no text content)';
}

/** Conversation detail — the message transcript for one conversation. */
export default async function ConversationDetailPage({ params }: { params: Promise<{ id: string }> }) {
    const { id } = await params;
    const decodedId = decodeURIComponent(id);

    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let data;
    try {
        data = await client.conversationMessages(decodedId);
    } catch (err) {
        return (
            <div>
                <Link href="/conversations" className="text-sm text-slate-400 hover:text-white">
                    ← Back to conversations
                </Link>
                <div className="mt-4">
                    <ErrorState error={err} />
                </div>
            </div>
        );
    }

    return (
        <div>
            <Link href="/conversations" className="text-sm text-slate-400 hover:text-white">
                ← Back to conversations
            </Link>
            <div className="mt-3">
                <PageHeader title="Conversation" subtitle={decodedId} />
            </div>

            {data.messages.length === 0 ? (
                <EmptyState message="No messages in this conversation." />
            ) : (
                <div className="space-y-3">
                    {data.messages.map((m) => {
                        const outbound = m.direction === 'outbound';
                        return (
                            <div key={m.id} className={`flex ${outbound ? 'justify-end' : 'justify-start'}`}>
                                <div className={`max-w-[75%] rounded-xl border px-4 py-3 ${outbound ? 'border-accent/30 bg-accent/10' : 'border-ink-700 bg-ink-850'}`}>
                                    <div className="mb-1 flex items-center gap-2 text-[11px] uppercase tracking-wider text-slate-500">
                                        <span>{m.from?.name ?? m.from?.type ?? m.direction}</span>
                                        <span>·</span>
                                        <span>{new Date(m.createdAt).toLocaleString()}</span>
                                    </div>
                                    <div className="whitespace-pre-wrap text-sm text-slate-200">{messageText(m)}</div>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}
        </div>
    );
}
