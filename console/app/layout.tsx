import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
    title: 'Smooth Operator — Console',
    description: 'Management console for the smooth-operator service.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
    return (
        <html lang="en">
            <body>{children}</body>
        </html>
    );
}
