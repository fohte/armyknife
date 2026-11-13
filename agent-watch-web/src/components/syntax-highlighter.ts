import { createHighlighterCore } from 'shiki/core'
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript'
import type { LanguageInput, TokensResult } from 'shiki/types'

type LanguageConfig = {
  id: string
  load: () => LanguageInput
}

const languageConfigs: Record<string, LanguageConfig> = {
  bash: { id: 'bash', load: () => import('shiki/langs/bash.mjs') },
  sh: { id: 'bash', load: () => import('shiki/langs/bash.mjs') },
  shell: { id: 'bash', load: () => import('shiki/langs/bash.mjs') },
  css: { id: 'css', load: () => import('shiki/langs/css.mjs') },
  diff: { id: 'diff', load: () => import('shiki/langs/diff.mjs') },
  patch: { id: 'diff', load: () => import('shiki/langs/diff.mjs') },
  html: { id: 'html', load: () => import('shiki/langs/html.mjs') },
  javascript: {
    id: 'javascript',
    load: () => import('shiki/langs/javascript.mjs'),
  },
  js: { id: 'javascript', load: () => import('shiki/langs/javascript.mjs') },
  jsx: { id: 'jsx', load: () => import('shiki/langs/jsx.mjs') },
  json: { id: 'json', load: () => import('shiki/langs/json.mjs') },
  markdown: { id: 'markdown', load: () => import('shiki/langs/markdown.mjs') },
  md: { id: 'markdown', load: () => import('shiki/langs/markdown.mjs') },
  python: { id: 'python', load: () => import('shiki/langs/python.mjs') },
  py: { id: 'python', load: () => import('shiki/langs/python.mjs') },
  rust: { id: 'rust', load: () => import('shiki/langs/rust.mjs') },
  rs: { id: 'rust', load: () => import('shiki/langs/rust.mjs') },
  sql: { id: 'sql', load: () => import('shiki/langs/sql.mjs') },
  toml: { id: 'toml', load: () => import('shiki/langs/toml.mjs') },
  ts: { id: 'typescript', load: () => import('shiki/langs/typescript.mjs') },
  typescript: {
    id: 'typescript',
    load: () => import('shiki/langs/typescript.mjs'),
  },
  tsx: { id: 'tsx', load: () => import('shiki/langs/tsx.mjs') },
  yaml: { id: 'yaml', load: () => import('shiki/langs/yaml.mjs') },
  yml: { id: 'yaml', load: () => import('shiki/langs/yaml.mjs') },
}

let highlighterPromise: ReturnType<typeof createHighlighterCore> | null = null
const languagePromises = new Map<string, Promise<void>>()

export async function highlightCode(code: string, language: string) {
  const config = languageConfigs[language.toLowerCase()]
  if (config === undefined) return null

  highlighterPromise ??= createHighlighterCore({
    themes: [import('shiki/themes/github-dark.mjs')],
    langs: [],
    engine: createJavaScriptRegexEngine(),
  }).catch((error: unknown) => {
    highlighterPromise = null
    return Promise.reject(
      error instanceof Error
        ? error
        : new Error('Failed to initialize the syntax highlighter.', {
            cause: error,
          }),
    )
  })
  const highlighter = await highlighterPromise

  let languagePromise = languagePromises.get(config.id)
  if (languagePromise === undefined) {
    languagePromise = highlighter.loadLanguage(config.load())
    languagePromises.set(config.id, languagePromise)
    void languagePromise.catch(() => {
      languagePromises.delete(config.id)
    })
  }
  await languagePromise

  return highlighter.codeToTokens(code, {
    lang: config.id,
    theme: 'github-dark',
  }) satisfies TokensResult
}
