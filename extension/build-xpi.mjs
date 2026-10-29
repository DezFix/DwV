#!/usr/bin/env node
/**
 * Builds an installable XPI for Firefox / LibreWolf.
 *
 * The XPI is a plain zip with manifest.json at the archive root. Firefox
 * refuses the package if the manifest is nested in a folder.
 *
 *   node extension/build-xpi.mjs
 */
import { existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const source = join(here, 'firefox')
const outDir = join(here, 'dist')
const manifestPath = join(source, 'manifest.json')

if (!existsSync(manifestPath)) {
  console.error(`No manifest at ${manifestPath}`)
  process.exit(1)
}

const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))

const version = manifest.version ?? '0.0.0'
const outFile = join(outDir, `dwv-${version}.xpi`)

mkdirSync(outDir, { recursive: true })

// Everything the extension ships, including `_locales`. The archive is built
// straight from the source folder, so manifest.json stays at the root.
const entries = readdirSync(source).filter(name => name !== 'manifest.json')

// zip it: `tar --format zip` (plain `tar -a` with an .xpi extension makes a tarball)
execFileSync(
  'tar',
  ['--format', 'zip', '-c', '-f', outFile, '-C', source, 'manifest.json', ...entries],
  { stdio: 'inherit' },
)

// Chrome has no load-from-file flow without the Web Store, so the same sources
// ship as a zip for "Load unpacked".
const chromeSource = join(here, 'chrome')
const chromeFile = join(outDir, `dwv-chrome-${version}.zip`)
const chromeEntries = readdirSync(chromeSource).filter(name => name !== 'manifest.json')

execFileSync(
  'tar',
  ['--format', 'zip', '-c', '-f', chromeFile, '-C', chromeSource, 'manifest.json', ...chromeEntries],
  { stdio: 'inherit' },
)

console.log(`Built ${outFile}`)
console.log(`Built ${chromeFile}`)
console.log('Install: about:debugging#/runtime/this-firefox -> Load Temporary Add-on -> pick the .xpi')
console.log('For a permanent install set xpinstall.signatures.required=false in about:config')
console.log('and use about:addons -> Install Add-on From File.')
console.log('Chrome: chrome://extensions -> enable Developer mode -> Load unpacked -> unpacked folder from the zip')
