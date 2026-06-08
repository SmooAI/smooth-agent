/** Role-rank helpers mirroring the Rust `role_rank` ordering. */

import type { Role } from './types';

const RANK: Record<Role, number> = { basic: 0, curator: 1, admin: 2 };

/** True when `role` meets at least `min` (Admin >= Curator >= Basic). */
export function hasRole(role: Role | undefined, min: Role): boolean {
    if (!role) return false;
    return RANK[role] >= RANK[min];
}

/** Curator-only surfaces (indexing, document sets) are hidden from Basic. */
export function canCurate(role: Role | undefined): boolean {
    return hasRole(role, 'curator');
}

/**
 * Admin-only mutations: create/edit/delete connectors and save settings. The
 * server enforces this too (`RequireRole<2>`); the UI hides/disables the controls
 * so a Curator never sees an action it can't perform.
 */
export function canManage(role: Role | undefined): boolean {
    return hasRole(role, 'admin');
}
