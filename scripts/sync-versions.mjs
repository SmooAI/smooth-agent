#!/usr/bin/env node
/**
 * Lockstep version sync.
 *
 * Changesets natively versions only the npm package (`@smooai/smooth-operator`). This script
 * stamps that canonical version onto every OTHER published-package manifest in the repo, so all
 * smooth-operator artifacts ship at one shared version (the @smooai/config model). It runs
 * automatically as part of `version:bump` (right after `changeset version`), and can be run
 * standalone to re-align.
 *
 * Add a target here when a new language package becomes publishable from this repo (a
 * Cargo.toml, a pyproject.toml, etc.).
 */
import { readFileSync, writeFileSync } from 'node:fs';

const anchorUrl = new URL('../typescript/package.json', import.meta.url);
const version = JSON.parse(readFileSync(anchorUrl, 'utf8')).version;

/** @type {{ name: string, url: URL, apply: (text: string) => string }[]} */
const targets = [
    {
        name: 'SmooAI.SmoothOperator.Core.csproj',
        url: new URL('../dotnet/core/src/SmooAI.SmoothOperator.Core.csproj', import.meta.url),
        apply: (text) => text.replace(/<Version>[^<]*<\/Version>/, `<Version>${version}</Version>`),
    },
];

let changed = 0;
for (const target of targets) {
    const before = readFileSync(target.url, 'utf8');
    const after = target.apply(before);
    if (after !== before) {
        writeFileSync(target.url, after);
        changed++;
        console.log(`synced ${version} → ${target.name}`);
    } else {
        console.log(`already at ${version}: ${target.name}`);
    }
}

console.log(`version-sync: anchor @smooai/smooth-operator@${version}, ${changed} file(s) updated.`);
