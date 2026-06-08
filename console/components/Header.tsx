import Image from 'next/image';
import type { Principal } from '@/lib/types';
import { RoleBadge } from './Badges';

/** Top bar: Smooth logo + app name + signed-in user/role + sign-out. */
export function Header({ principal }: { principal: Principal | undefined }) {
    return (
        <header className="flex h-16 shrink-0 items-center justify-between border-b border-ink-800 bg-ink-900 px-6">
            <div className="flex items-center gap-3">
                <Image src="/smooth-logo.svg" alt="Smooth" width={32} height={32} priority />
                <div className="leading-tight">
                    <div className="text-sm font-semibold text-white">Smooth Operator</div>
                    <div className="text-[11px] uppercase tracking-wider text-slate-500">Management Console</div>
                </div>
            </div>

            <div className="flex items-center gap-4">
                {principal ? (
                    <div className="flex items-center gap-3">
                        <div className="text-right leading-tight">
                            <div className="text-sm font-medium text-white">{principal.displayName ?? principal.userId}</div>
                            <div className="text-[11px] text-slate-500">{principal.orgId}</div>
                        </div>
                        <RoleBadge role={principal.role} />
                    </div>
                ) : null}
                <form action="/api/auth/signout" method="post">
                    <button type="submit" className="btn-ghost">
                        Sign out
                    </button>
                </form>
            </div>
        </header>
    );
}
