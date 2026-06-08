'use client';

import { useState, useTransition } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { deleteConnectorAction, indexConnectorAction } from '@/app/(app)/connectors/actions';
import { StatusBadge } from '@/components/Badges';
import type { IndexingRun } from '@/lib/types';

/**
 * Per-row actions for a connector: Edit (Admin), "Index now" (Curator+), Delete
 * (Admin, with a confirm). "Index now" surfaces the returned run's status/counts
 * inline and links to the full `/indexing` table. `canManage` hides the
 * Admin-only mutations from a Curator (the server enforces this regardless).
 */
export function ConnectorRowActions({ id, name, canManage }: { id: string; name: string; canManage: boolean }) {
    const router = useRouter();
    const [pending, startTransition] = useTransition();
    const [run, setRun] = useState<IndexingRun | null>(null);
    const [error, setError] = useState<string | null>(null);

    function onIndex() {
        setError(null);
        startTransition(async () => {
            const res = await indexConnectorAction(id);
            if (res.ok) {
                setRun(res.run);
                router.refresh();
            } else {
                setError(res.error);
            }
        });
    }

    function onDelete() {
        if (!confirm(`Delete connector "${name}"? This cannot be undone.`)) return;
        setError(null);
        startTransition(async () => {
            const res = await deleteConnectorAction(id);
            if (res.ok) {
                router.refresh();
            } else {
                setError(res.error);
            }
        });
    }

    return (
        <div className="flex flex-col items-end gap-1.5">
            <div className="flex items-center gap-2">
                <button type="button" onClick={onIndex} disabled={pending} className="btn-ghost" data-testid="index-now">
                    {pending ? '…' : 'Index now'}
                </button>
                {canManage ? (
                    <>
                        <Link href={`/connectors/${encodeURIComponent(id)}/edit`} className="btn-ghost">
                            Edit
                        </Link>
                        <button type="button" onClick={onDelete} disabled={pending} className="btn-danger" data-testid="delete-connector">
                            Delete
                        </button>
                    </>
                ) : null}
            </div>

            {run ? (
                <div className="flex items-center gap-2 text-xs text-slate-400" data-testid="index-result">
                    <StatusBadge status={run.status} />
                    <span>
                        {run.documentsSeen} seen · {run.chunksIndexed} indexed
                    </span>
                    <Link href="/indexing" className="text-accent-soft hover:underline">
                        view run →
                    </Link>
                </div>
            ) : null}
            {run?.error ? <div className="text-xs text-rose-400">{run.error}</div> : null}
            {error ? <div className="text-xs text-rose-400">{error}</div> : null}
        </div>
    );
}
