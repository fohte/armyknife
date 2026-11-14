import { readdir, readFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { expect, it } from 'vitest'

const sourceDirectory = path.dirname(fileURLToPath(import.meta.url))
const resolvePackageFile = createRequire(import.meta.url).resolve

async function cssFilesIn(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const nestedFiles = await Promise.all(
    entries.map(async (entry) => {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) return cssFilesIn(entryPath)
      return entry.isFile() && entry.name.endsWith('.css') ? [entryPath] : []
    }),
  )
  return nestedFiles.flat()
}

it('uses only defined --aw-* custom properties in application stylesheets', async () => {
  const uiTokensPath = resolvePackageFile('@fohte/ui/tokens.css')
  const cssPaths = [...(await cssFilesIn(sourceDirectory)), uiTokensPath].sort()
  const stylesheets = await Promise.all(
    cssPaths.map(async (filePath) => ({
      filePath,
      content: await readFile(filePath, 'utf8'),
      label:
        filePath === uiTokensPath
          ? '@fohte/ui/tokens.css'
          : `src/${path.relative(sourceDirectory, filePath)}`,
    })),
  )
  const definitions = new Set(
    stylesheets.flatMap(({ content }) =>
      Array.from(content.matchAll(/(--aw-[\w-]+)\s*:/g), ([, name]) => name),
    ),
  )
  const unresolved = stylesheets.flatMap(({ content, label }) =>
    Array.from(content.matchAll(/var\(\s*(--aw-[\w-]+)/g), ([, name]) => name)
      .filter((name) => !definitions.has(name))
      .map((variable) => ({ file: label, variable })),
  )

  expect(unresolved).toEqual([])
})
