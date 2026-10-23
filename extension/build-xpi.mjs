#!/usr/bin/env node
/**
 * Builds an installable XPI for Firefox / LibreWolf.
 *
 * The XPI is a plain zip with manifest.json at the archive root. Firefox
 * refuses the package if the manifest is nested in a folder.
 *
 *   node extension/build-xpi.mjs
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync } from 'node:fs'
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

// Only the files the extension actually ships.
const files = readdirSync(source).filter(name => {
  const full = join(source, name)
  return statSync(full).isFile() && name !== 'manifest.json'
})

const tmp = join(outDir, `.stage-${version}`)
if (existsSync(tmp)) {
  execFileSync('cmd', ['/c', 'rmdir', '/s', '/q', tmp])
}
mkdirSync(tmp, { recursive: true })

execFileSync('cmd', ['/c', 'copy', '/y', manifestPath, join(tmp, 'manifest.json')])
for (const name of files) {
  execFileSync('cmd', ['/c', 'copy', '/y', join(source, name), join(tmp, name)])
}

// zip it: `tar --format zip` (plain `tar -a` with an .xpi extension makes a tarball)
execFileSync(
  'tar',
  ['--format', 'zip', '-c', '-f', outFile, '-C', tmp, 'manifest.json', ...files],
  { stdio: 'inherit' },
)

execFileSync('cmd', ['/c', 'rmdir', '/s', '/q', tmp])

console.log(`Built ${outFile}`)
console.log('Install: about:debugging#/runtime/this-firefox -> Load Temporary Add-on -> pick the .xpi')
console.log('For a permanent install set xpinstall.signatures.required=false in about:config')
console.log('and use about:addons -> Install Add-on From File.')
