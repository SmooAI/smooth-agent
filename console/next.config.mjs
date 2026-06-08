/** @type {import('next').NextConfig} */
const nextConfig = {
    reactStrictMode: true,
    // The console is a pure read client of the admin API; no image optimization
    // backend is needed. Keep the build self-contained.
    images: { unoptimized: true },
};

export default nextConfig;
