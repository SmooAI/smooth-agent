'use server';

import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import { AdminApiError } from '@/lib/admin-client';
import { getAdminClient } from '@/lib/session';
import type { ConnectorKind, ConnectorWrite, IndexingRun } from '@/lib/types';

/**
 * Server actions backing the connector write pages.
 *
 * Mutations return a discriminated result (`{ ok }` / `{ error }`) so the client
 * form can surface a backend `400 VALIDATION_ERROR` message inline rather than
 * throwing an unhandled error. The admin API re-enforces the Admin role on every
 * write, so a Curator that somehow POSTs gets a clean `403` surfaced here.
 *
 * `auth_ref` is collected as a secret **name** (e.g. `GITHUB_TOKEN`) — never a
 * token value — matching the backend's `auth_ref` secret model.
 */

export type ActionResult = { ok: true } | { ok: false; error: string };

/** Build a kind-specific `config` payload from raw form values. */
function buildConfig(kind: ConnectorKind, form: FormData): Record<string, unknown> {
    if (kind === 'github') {
        const config: Record<string, unknown> = {
            owner: String(form.get('owner') ?? '').trim(),
            repo: String(form.get('repo') ?? '').trim(),
            include: {
                prose: form.get('include_prose') === 'on',
                code: form.get('include_code') === 'on',
                issues: form.get('include_issues') === 'on',
            },
        };
        const ref = String(form.get('ref') ?? '').trim();
        if (ref) config.ref = ref;
        const visibility = String(form.get('visibility') ?? '').trim();
        if (visibility === 'public' || visibility === 'private') config.visibility = visibility;
        // auth_ref is a secret NAME, never a token value.
        const authRef = String(form.get('auth_ref') ?? '').trim();
        if (authRef) config.auth_ref = authRef;
        return config;
    }
    if (kind === 'web') {
        return { url: String(form.get('url') ?? '').trim() };
    }
    // file
    return { path: String(form.get('path') ?? '').trim() };
}

function parseWrite(form: FormData): ConnectorWrite {
    const kind = String(form.get('kind') ?? '') as ConnectorKind;
    return {
        name: String(form.get('name') ?? '').trim(),
        kind,
        config: buildConfig(kind, form),
        enabled: form.get('enabled') === 'on',
    };
}

function describe(err: unknown): string {
    if (err instanceof AdminApiError) {
        if (err.isForbidden) return 'Your role does not permit this change (Admin required).';
        return err.message;
    }
    return err instanceof Error ? err.message : String(err);
}

/** Create a connector, then redirect to the list on success. */
export async function createConnectorAction(_prev: ActionResult | null, form: FormData): Promise<ActionResult> {
    const client = await getAdminClient();
    if (!client) return { ok: false, error: 'Not signed in.' };
    try {
        await client.createConnector(parseWrite(form));
    } catch (err) {
        return { ok: false, error: describe(err) };
    }
    revalidatePath('/connectors');
    redirect('/connectors');
}

/** Update a connector, then redirect to the list on success. */
export async function updateConnectorAction(id: string, _prev: ActionResult | null, form: FormData): Promise<ActionResult> {
    const client = await getAdminClient();
    if (!client) return { ok: false, error: 'Not signed in.' };
    try {
        await client.updateConnector(id, parseWrite(form));
    } catch (err) {
        return { ok: false, error: describe(err) };
    }
    revalidatePath('/connectors');
    revalidatePath(`/connectors/${id}/edit`);
    redirect('/connectors');
}

/** Delete a connector (called from the list's row action). */
export async function deleteConnectorAction(id: string): Promise<ActionResult> {
    const client = await getAdminClient();
    if (!client) return { ok: false, error: 'Not signed in.' };
    try {
        await client.deleteConnector(id);
    } catch (err) {
        return { ok: false, error: describe(err) };
    }
    revalidatePath('/connectors');
    return { ok: true };
}

export type IndexResult = { ok: true; run: IndexingRun } | { ok: false; error: string };

/** Trigger "Index now" for a connector and return the resulting run. */
export async function indexConnectorAction(id: string): Promise<IndexResult> {
    const client = await getAdminClient();
    if (!client) return { ok: false, error: 'Not signed in.' };
    try {
        const run = await client.indexConnector(id);
        revalidatePath('/indexing');
        revalidatePath('/connectors');
        return { ok: true, run };
    } catch (err) {
        return { ok: false, error: describe(err) };
    }
}
