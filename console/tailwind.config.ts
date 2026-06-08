import type { Config } from 'tailwindcss';

const config: Config = {
    content: ['./app/**/*.{ts,tsx}', './components/**/*.{ts,tsx}'],
    theme: {
        extend: {
            colors: {
                // Smooth brand palette — deep slate canvas with a teal/cyan accent
                // matching the smooth-web logo.
                ink: {
                    950: '#0a0e14',
                    900: '#0f141d',
                    850: '#141b27',
                    800: '#1a2231',
                    700: '#232d40',
                    600: '#2f3b52',
                },
                accent: {
                    DEFAULT: '#2dd4bf',
                    soft: '#5eead4',
                    dim: '#0d9488',
                },
            },
            fontFamily: {
                sans: ['var(--font-sans)', 'system-ui', 'sans-serif'],
                mono: ['ui-monospace', 'SFMono-Regular', 'monospace'],
            },
        },
    },
    plugins: [],
};

export default config;
