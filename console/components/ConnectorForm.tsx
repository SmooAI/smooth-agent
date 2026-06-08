'use client';

import { useActionState, useState } from 'react';
import { useFormStatus } from 'react-dom';
import Link from 'next/link';
import type { ActionResult } from '@/app/(app)/connectors/actions';
import type { ConnectorConfig, ConnectorKind } from '@/lib/types';

/** Read a nested config value as a string for prefilling the edit form. */
function str(config: Record<string, unknown>, key: string): string {
    const v = config[key];
    return typeof v === 'string' ? v : '';
}

/** Read a github `include` toggle (defaults to true when absent on create). */
function include(config: Record<string, unknown>, key: 'prose' | 'code' | 'issues', fallback: boolean): boolean {
    const inc = config.include;
    if (inc && typeof inc === 'object' && key in inc) {
        return Boolean((inc as Record<string, unknown>)[key]);
    }
    return fallback;
}

function Submit({ label }: { label: string }) {
    const { pending } = useFormStatus();
    return (
        <button type="submit" className="btn" disabled={pending}>
            {pending ? 'Saving…' : label}
        </button>
    );
}

/**
 * Add/edit form for a connector config. Picks a `kind`, then renders the
 * kind-specific fields. Used by `/connectors/new` and `/connectors/[id]/edit`,
 * wired to the matching server action. Validation `400`s come back as an inline
 * error (the backend re-checks every write).
 */
export function ConnectorForm({
    action,
    existing,
    submitLabel,
}: {
    action: (prev: ActionResult | null, form: FormData) => Promise<ActionResult>;
    existing?: ConnectorConfig;
    submitLabel: string;
}) {
    const [state, formAction] = useActionState(action, null);
    const [kind, setKind] = useState<ConnectorKind>(existing?.kind ?? 'github');
    const config = existing?.config ?? {};

    return (
        <form action={formAction} className="card max-w-2xl space-y-5">
            {state && !state.ok ? (
                <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 px-4 py-3 text-sm text-rose-300" role="alert">
                    {state.error}
                </div>
            ) : null}

            <Field label="Name">
                <input name="name" required defaultValue={existing?.name ?? ''} placeholder="Docs repo" className="input" data-testid="connector-name" />
            </Field>

            <Field label="Kind">
                <select
                    name="kind"
                    value={kind}
                    onChange={(e) => setKind(e.target.value as ConnectorKind)}
                    className="input"
                    data-testid="connector-kind"
                    // The kind is immutable on edit (it changes the config schema);
                    // creating a new connector is the way to switch kinds.
                    disabled={Boolean(existing)}
                >
                    <option value="github">GitHub</option>
                    <option value="web">Web</option>
                    <option value="file">File</option>
                </select>
                {existing ? <input type="hidden" name="kind" value={kind} /> : null}
            </Field>

            {kind === 'github' ? (
                <>
                    <div className="grid grid-cols-2 gap-4">
                        <Field label="Owner">
                            <input name="owner" required defaultValue={str(config, 'owner')} placeholder="smooai" className="input" />
                        </Field>
                        <Field label="Repo">
                            <input name="repo" required defaultValue={str(config, 'repo')} placeholder="docs" className="input" />
                        </Field>
                    </div>
                    <Field label="Include">
                        <div className="flex flex-wrap gap-4 pt-1">
                            <Toggle name="include_prose" label="Prose" defaultChecked={include(config, 'prose', true)} />
                            <Toggle name="include_code" label="Code" defaultChecked={include(config, 'code', true)} />
                            <Toggle name="include_issues" label="Issues" defaultChecked={include(config, 'issues', false)} />
                        </div>
                    </Field>
                    <div className="grid grid-cols-2 gap-4">
                        <Field label="Ref (optional)">
                            <input name="ref" defaultValue={str(config, 'ref')} placeholder="main" className="input" />
                        </Field>
                        <Field label="Visibility">
                            <select name="visibility" defaultValue={str(config, 'visibility') || 'public'} className="input">
                                <option value="public">Public</option>
                                <option value="private">Private</option>
                            </select>
                        </Field>
                    </div>
                    <Field label="Auth ref (secret name — optional)" hint="The NAME of a secret/env var (e.g. GITHUB_TOKEN), never the token itself. Resolved at index time.">
                        <input name="auth_ref" defaultValue={str(config, 'auth_ref')} placeholder="GITHUB_TOKEN" className="input font-mono" data-testid="connector-auth-ref" />
                    </Field>
                </>
            ) : null}

            {kind === 'web' ? (
                <Field label="URL">
                    <input name="url" required type="url" defaultValue={str(config, 'url')} placeholder="https://example.com/docs" className="input" />
                </Field>
            ) : null}

            {kind === 'file' ? (
                <Field label="Path" hint="A local file or directory the server can read.">
                    <input name="path" required defaultValue={str(config, 'path')} placeholder="/var/data/docs" className="input font-mono" data-testid="connector-path" />
                </Field>
            ) : null}

            <Field label="Enabled">
                <Toggle name="enabled" label="Active" defaultChecked={existing?.enabled ?? true} />
            </Field>

            <div className="flex items-center gap-3 pt-2">
                <Submit label={submitLabel} />
                <Link href="/connectors" className="btn-ghost">
                    Cancel
                </Link>
            </div>
        </form>
    );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
    return (
        <label className="block">
            <span className="mb-1.5 block text-sm font-medium text-slate-300">{label}</span>
            {children}
            {hint ? <span className="mt-1 block text-xs text-slate-500">{hint}</span> : null}
        </label>
    );
}

function Toggle({ name, label, defaultChecked }: { name: string; label: string; defaultChecked: boolean }) {
    return (
        <label className="inline-flex items-center gap-2 text-sm text-slate-300">
            <input type="checkbox" name={name} defaultChecked={defaultChecked} className="h-4 w-4 rounded border-ink-600 bg-ink-900 accent-accent" />
            {label}
        </label>
    );
}
