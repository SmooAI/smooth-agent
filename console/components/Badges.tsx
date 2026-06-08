import type { IndexingRunStatus, Role } from '@/lib/types';

const ROLE_STYLE: Record<Role, string> = {
    admin: 'bg-accent/15 text-accent-soft',
    curator: 'bg-sky-500/15 text-sky-300',
    basic: 'bg-slate-500/15 text-slate-300',
};

export function RoleBadge({ role }: { role: Role }) {
    return <span className={`badge ${ROLE_STYLE[role]}`}>{role}</span>;
}

const STATUS_STYLE: Record<IndexingRunStatus, string> = {
    succeeded: 'bg-emerald-500/15 text-emerald-300',
    running: 'bg-amber-500/15 text-amber-300',
    failed: 'bg-rose-500/15 text-rose-300',
};

export function StatusBadge({ status }: { status: IndexingRunStatus }) {
    const cls = STATUS_STYLE[status] ?? 'bg-slate-500/15 text-slate-300';
    return <span className={`badge ${cls}`}>{status}</span>;
}
