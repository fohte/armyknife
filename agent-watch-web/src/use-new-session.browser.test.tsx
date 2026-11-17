import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

import { type NewSessionState, useNewSession } from '#use-new-session'

let container: HTMLDivElement
let root: Root | null = null
let currentState: NewSessionState | null = null

function NewSessionHarness() {
  currentState = useNewSession(true)
  return null
}

function requestUrl(input: RequestInfo | URL) {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.href
  return input.url
}

async function flushRequests() {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
}

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  currentState = null
  container.remove()
  vi.unstubAllGlobals()
})

it('reloads skill suggestions when the selected engine and directory change', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const fetcher = vi.fn<typeof fetch>((input) => {
    const query = new URL(requestUrl(input), window.location.href).searchParams
    const engine = query.get('engine') ?? 'unknown'
    const directory = query.get('directory') ?? 'recent'
    return Promise.resolve(
      new Response(
        JSON.stringify({
          skills: [
            { name: `${engine}-${directory}`, description: 'A fixture skill.' },
          ],
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  const recentSkills = currentState?.skillSuggestions

  act(() => currentState?.setEngine('codex'))
  await flushRequests()
  const codexSkills = currentState?.skillSuggestions

  act(() =>
    currentState?.chooseDirectory({
      path: '/fixture/project',
      source: 'recent',
    }),
  )
  await flushRequests()

  const snapshot = () => ({
    recentSkills,
    codexSkills,
    selectedDirectorySkills: currentState?.skillSuggestions,
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
  })
  expect(snapshot()).toEqual({
    recentSkills: [{ name: 'claude-recent', description: 'A fixture skill.' }],
    codexSkills: [{ name: 'codex-recent', description: 'A fixture skill.' }],
    selectedDirectorySkills: [
      {
        name: 'codex-/fixture/project',
        description: 'A fixture skill.',
      },
    ],
    requests: [
      '/api/skill-suggestions?engine=claude',
      '/api/skill-suggestions?engine=codex',
      '/api/skill-suggestions?engine=codex&directory=%2Ffixture%2Fproject',
    ],
  })
})

it('keeps the current suggestions when an earlier request resolves last', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const pendingRequests: {
    url: string
    resolve: (response: Response) => void
  }[] = []
  const fetcher = vi.fn<typeof fetch>((input) => {
    return new Promise((resolve) => {
      pendingRequests.push({
        url: requestUrl(input),
        resolve,
      })
    })
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  act(() => currentState?.setEngine('codex'))

  const resolveRequest = async (index: number, name: string) => {
    const request = pendingRequests[index]
    if (request === undefined)
      throw new Error('Expected a pending skill request')
    await act(async () => {
      request.resolve(
        new Response(
          JSON.stringify({
            skills: [{ name, description: 'A fixture skill.' }],
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      )
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    })
  }

  await resolveRequest(1, 'codex-fixture-skill')
  await resolveRequest(0, 'claude-fixture-skill')

  const snapshot = () => ({
    skills: currentState?.skillSuggestions,
    requests: pendingRequests.map(({ url }) => url),
  })
  expect(snapshot()).toEqual({
    skills: [{ name: 'codex-fixture-skill', description: 'A fixture skill.' }],
    requests: [
      '/api/skill-suggestions?engine=claude',
      '/api/skill-suggestions?engine=codex',
    ],
  })
})
