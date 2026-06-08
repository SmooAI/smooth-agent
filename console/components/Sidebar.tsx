'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import type { Role } from '@/lib/types';
import { canCurate } from '@/lib/rbac';

interface NavItem {
    href: string;
    label: string;
    icon: string;
    /** When true, only shown to Curator+ (hidden from Basic). */
    curatorOnly?: boolean;
}

const NAV: NavItem[] = [
    { href: '/', label: 'Dashboard', icon: '▣' },
    { href: '/conversations', label: 'Conversations', icon: '💬' },
    { href: '/indexing', label: 'Indexing', icon: '⟳', curatorOnly: true },
    { href: '/document-sets', label: 'Document Sets', icon: '🗂' },
    { href: '/settings', label: 'Settings', icon: '⚙' },
];

export function Sidebar({ role }: { role: Role | undefined }) {
    const pathname = usePathname();
    const items = NAV.filter((i) => !i.curatorOnly || canCurate(role));

    return (
        <nav className="flex w-60 shrink-0 flex-col gap-1 border-r border-ink-800 bg-ink-900 p-3">
            {items.map((item) => {
                const active = item.href === '/' ? pathname === '/' : pathname.startsWith(item.href);
                return (
                    <Link key={item.href} href={item.href} className={`nav-link ${active ? 'nav-link-active' : ''}`}>
                        <span aria-hidden className="w-5 text-center text-base">
                            {item.icon}
                        </span>
                        {item.label}
                    </Link>
                );
            })}
        </nav>
    );
}
