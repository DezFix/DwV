#!/usr/bin/env node
/**
 * One source for both browsers: everything in `firefox/` except the manifest
 * is copied to `chrome/`. Only `chrome/manifest.json` is edited by hand
 * (service worker instead of background scripts, no webRequestBlocking).
 *
 *   node extension/sync.mjs          copy firefox/ -> chrome/
 *   node extension/sync.mjs --check  fail if chrome/ has drifted (CI)
 */
import {
  copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync,
} from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const source = join(here, 'firefox')
const target = join(here, 'chrome')
const check = process.argv.includes('--check')
const OWN = new Set(['manifest.json'])

function walk(dir, base = dir) {
  const out = []
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    const rel = relative(base, path)
    if (statSync(path).isDirectory()) out.push(...walk(path, base))
    else if (!OWN.has(rel)) out.push(rel)
  }
  return out.sort()
}

const wanted = walk(source)
const present = existsSync(target) ? walk(target) : []
const drift = []

for (const rel of wanted) {
  const from = join(source, rel)
  const to = join(target, rel)
  const same = existsSync(to) && readFileSync(from).equals(readFileSync(to))
  if (same) continue
  drift.push(rel)
  if (!check) {
    mkdirSync(dirname(to), { recursive: true })
    copyFileSync(from, to)
  }
}
for (const rel of present) {
  if (wanted.includes(rel)) continue
  drift.push(`${rel} (only in chrome/)`)
  if (!check) rmSync(join(target, rel))
}

// The two packages ship together, so their versions must match.
const sourceManifest = JSON.parse(readFileSync(join(source, 'manifest.json'), 'utf8'))
const targetManifestPath = join(target, 'manifest.json')
const targetManifest = JSON.parse(readFileSync(targetManifestPath, 'utf8'))
if (targetManifest.version !== sourceManifest.version) {
  drift.push(`manifest.json version ${targetManifest.version} != ${sourceManifest.version}`)
  if (!check) {
    targetManifest.version = sourceManifest.version
    writeFileSync(targetManifestPath, `${JSON.stringify(targetManifest, null, 2)}\n`)
  }
}

if (check) {
  if (drift.length > 0) {
    console.error('extension/chrome differs from extension/firefox:')
    for (const rel of drift) console.error(`  ${rel}`)
    console.error('Edit extension/firefox and run: node extension/sync.mjs')
    process.exit(1)
  }
  console.log('extension/chrome is in sync')
} else {
  console.log(drift.length ? `synced ${drift.length} file(s) to chrome/` : 'chrome/ already in sync')
}
