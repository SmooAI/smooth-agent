'use client';

import { useActionState } from 'react';
import { useFormStatus } from 'react-dom';
import { saveSettingsAction } from '@/app/(app)/settings/actions';
import type { AgentSettings } from '@/lib/types';

function Submit() {
    const { pending } = useFormStatus();
    return (
        <button type="submit" className="btn" disabled={pending} data-testid="save-settings">
            {pending ? 'Saving…' : 'Save settings'}
        </button>
    );
}

/**
 * Editable agent-settings form (Admin only — `canManage`). A Curator gets the
 * read-only view (the inputs are disabled and the save button hidden); the
 * backend re-enforces Admin on `PUT /admin/settings`.
 */
export function SettingsForm({ settings, canManage }: { settings: AgentSettings; canManage: boolean }) {
    const [state, formAction] = useActionState(saveSettingsAction, null);

    return (
        <form action={formAction} className="card max-w-2xl space-y-5">
            {state?.ok ? (
                <div className="rounded-lg border border-emerald-500/30 bg-emerald-500/5 px-4 py-3 text-sm text-emerald-300" role="status">
                    Settings saved.
                </div>
            ) : null}
            {state && !state.ok ? (
                <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 px-4 py-3 text-sm text-rose-300" role="alert">
                    {state.error}
                </div>
            ) : null}

            <label className="block">
                <span className="mb-1.5 block text-sm font-medium text-slate-300">Model</span>
                <input name="model" required defaultValue={settings.model} disabled={!canManage} className="input font-mono" data-testid="settings-model" />
            </label>

            <label className="block">
                <span className="mb-1.5 block text-sm font-medium text-slate-300">System prompt</span>
                <textarea name="systemPrompt" rows={6} defaultValue={settings.systemPrompt} disabled={!canManage} className="input resize-y font-mono text-xs leading-relaxed" />
            </label>

            <label className="block">
                <span className="mb-1.5 block text-sm font-medium text-slate-300">Default tools</span>
                <textarea
                    name="defaultTools"
                    rows={3}
                    defaultValue={settings.defaultTools.join('\n')}
                    disabled={!canManage}
                    placeholder="One tool name per line (or comma-separated)"
                    className="input resize-y font-mono text-xs"
                    data-testid="settings-tools"
                />
                <span className="mt-1 block text-xs text-slate-500">One tool name per line; blanks and duplicates are dropped.</span>
            </label>

            <div className="flex items-center gap-3 pt-1 text-xs text-slate-500">
                <span>Last updated {new Date(settings.updatedAt).toLocaleString()}</span>
            </div>

            {canManage ? (
                <div className="pt-1">
                    <Submit />
                </div>
            ) : (
                <div className="rounded-lg border border-ink-700 bg-ink-900 px-4 py-3 text-xs text-slate-400">
                    Editing agent settings requires the Admin role. Your role can view these values.
                </div>
            )}
        </form>
    );
}
