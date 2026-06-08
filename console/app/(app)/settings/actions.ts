'use server';

import { revalidatePath } from 'next/cache';
import { AdminApiError } from '@/lib/admin-client';
import { getAdminClient } from '@/lib/session';

/**
 * Server action backing the editable agent-settings form (`PUT /admin/settings`,
 * Admin only). Returns a result the client form renders inline; the admin API
 * re-enforces the Admin role, so a Curator that POSTs gets a clean `403` here.
 *
 * `defaultTools` is collected as a newline/comma-separated list and normalized to
 * a deduped string array.
 */

export type SettingsResult = { ok: true } | { ok: false; error: string };

function parseTools(raw: string): string[] {
    const seen = new Set<string>();
    for (const tok of raw.split(/[\n,]/)) {
        const t = tok.trim();
        if (t) seen.add(t);
    }
    return [...seen];
}

export async function saveSettingsAction(_prev: SettingsResult | null, form: FormData): Promise<SettingsResult> {
    const client = await getAdminClient();
    if (!client) return { ok: false, error: 'Not signed in.' };

    const model = String(form.get('model') ?? '').trim();
    const systemPrompt = String(form.get('systemPrompt') ?? '');
    const defaultTools = parseTools(String(form.get('defaultTools') ?? ''));

    if (!model) return { ok: false, error: 'Model is required.' };

    try {
        await client.putSettings({ model, systemPrompt, defaultTools });
    } catch (err) {
        if (err instanceof AdminApiError) {
            if (err.isForbidden) return { ok: false, error: 'Saving settings requires the Admin role.' };
            return { ok: false, error: err.message };
        }
        return { ok: false, error: err instanceof Error ? err.message : String(err) };
    }

    revalidatePath('/settings');
    return { ok: true };
}
