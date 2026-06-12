# Changesets

This repo releases **all published artifacts in lockstep** off Changesets — one
shared `smooth-operator` version across npm (`@smooai/smooth-operator`) and NuGet
(`SmooAI.SmoothOperator.Core`), with future language packages added the same way.

## Cutting a release

1. With your change, add a changeset describing the bump:
   ```bash
   pnpm changeset
   ```
   Pick `@smooai/smooth-operator` and the semver level. (It's the version anchor;
   the bump is mirrored onto every other artifact by `scripts/sync-versions.mjs`.)
2. Merge your PR (the changeset rides along).
3. The **Release (Changesets 🦋)** workflow opens a "🦋 New version release" PR that
   bumps the npm version and stamps the same version onto the .NET csproj.
4. Merging that PR publishes **npm + NuGet** at the one shared version.

`scripts/sync-versions.mjs` is where you register a new publishable language
package (Cargo.toml, pyproject.toml, …) so it joins the lockstep.

See <https://github.com/changesets/changesets> for the tool itself.
