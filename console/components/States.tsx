import { AdminApiError } from '@/lib/admin-client';

/** A page-level heading with an optional subtitle. */
export function PageHeader({ title, subtitle }: { title: string; subtitle?: string }) {
    return (
        <div className="mb-6">
            <h1 className="text-2xl font-semibold text-white">{title}</h1>
            {subtitle ? <p className="mt-1 text-sm text-slate-400">{subtitle}</p> : null}
        </div>
    );
}

/** Empty-state placeholder for a section with no data. */
export function EmptyState({ message }: { message: string }) {
    return (
        <div className="card flex flex-col items-center justify-center py-12 text-center">
            <div className="text-3xl opacity-40">∅</div>
            <p className="mt-3 text-sm text-slate-400">{message}</p>
        </div>
    );
}

/**
 * Error state. Translates admin-API auth failures into friendly copy so a
 * Basic user hitting a Curator-only surface sees "insufficient permissions"
 * rather than a raw 403.
 */
export function ErrorState({ error }: { error: unknown }) {
    let title = 'Something went wrong';
    let detail = error instanceof Error ? error.message : String(error);

    if (error instanceof AdminApiError) {
        if (error.isForbidden) {
            title = 'Insufficient permissions';
            detail = 'Your role does not grant access to this section.';
        } else if (error.isUnauthorized) {
            title = 'Not authenticated';
            detail = 'Your session is invalid or expired. Sign in again.';
        } else if (error.isNotFound) {
            title = 'Not found';
        }
    }

    return (
        <div className="card border-rose-500/30 bg-rose-500/5">
            <div className="text-sm font-semibold text-rose-300">{title}</div>
            <p className="mt-1 text-sm text-slate-400">{detail}</p>
        </div>
    );
}

/** A simple skeleton block for loading states. */
export function Skeleton({ rows = 3 }: { rows?: number }) {
    return (
        <div className="card space-y-3">
            {Array.from({ length: rows }).map((_, i) => (
                <div key={i} className="h-4 animate-pulse rounded bg-ink-700" style={{ width: `${70 + ((i * 13) % 25)}%` }} />
            ))}
        </div>
    );
}
