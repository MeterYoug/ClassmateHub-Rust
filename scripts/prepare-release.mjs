import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { copyFile, mkdir, readdir, readFile, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

const [bundlesDir, outputDir] = process.argv.slice(2)
if (!bundlesDir || !outputDir) {
  throw new Error('Usage: node scripts/prepare-release.mjs <bundles-dir> <new-output-dir>')
}

const { version } = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'))
const downloadBase = `https://github.com/MeterYoug/ClassmateHub-Releases/releases/download/v${version}`
const maxAssetBytes = 600 * 1024 * 1024
const targets = [
  { platform: 'windows-x86_64', extension: '.exe', suffix: '-setup.exe' },
  { platform: 'darwin-aarch64', extension: '.dmg', suffix: '.dmg' },
  { platform: 'linux-x86_64', extension: '.deb', suffix: '.deb' }
]

const installers = []
for (const { platform, extension, suffix } of targets) {
  const bundleDir = join(bundlesDir, `bundle-${platform}`)
  const entries = await readdir(bundleDir, { withFileTypes: true })
  const matches = entries.filter((entry) => entry.isFile() && entry.name.endsWith(extension))
  if (matches.length !== 1 || !matches[0].name.includes(`_${version}_`)) {
    throw new Error(`Expected one ${extension} installer for ${platform} at version ${version}`)
  }
  const source = join(bundleDir, matches[0].name)
  const { size } = await stat(source)
  if (size === 0 || size > maxAssetBytes) {
    throw new Error(`Installer size for ${platform} must be between 1 and ${maxAssetBytes} bytes`)
  }
  installers.push({ source, platform, name: `ClassmateHub_${version}_${platform}${suffix}` })
}

await mkdir(outputDir)
const platforms = {}
const checksums = []
for (const { source, platform, name } of installers) {
  const output = join(outputDir, name)
  await copyFile(source, output)
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(output)) {
    hash.update(chunk)
  }
  platforms[platform] = { url: `${downloadBase}/${name}` }
  checksums.push(`${hash.digest('hex')}  ${name}\n`)
}
await writeFile(join(outputDir, 'latest.json'), `${JSON.stringify({ version, platforms }, null, 2)}\n`, 'utf8')
await writeFile(join(outputDir, 'SHA256SUMS.txt'), checksums.join(''), 'utf8')
