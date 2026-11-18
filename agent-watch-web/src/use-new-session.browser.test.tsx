import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

import { exampleModelOptions } from '#new-session-story-fixtures.test'
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
    const url = new URL(requestUrl(input), window.location.href)
    const engine = query.get('engine') ?? 'unknown'
    const directory = query.get('directory') ?? 'recent'
    return Promise.resolve(
      new Response(
        JSON.stringify(
          url.pathname === '/api/model-options'
            ? exampleModelOptions
            : {
                skills: [
                  {
                    name: `${engine}-${directory}`,
                    description: 'A fixture skill.',
                  },
                ],
              },
        ),
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
      '/api/model-options?engine=claude',
      '/api/skill-suggestions?engine=claude',
      '/api/model-options?engine=codex',
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
    const url = requestUrl(input)
    if (url.startsWith('/api/model-options')) {
      return Promise.resolve(
        new Response(JSON.stringify(exampleModelOptions), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }
    return new Promise((resolve) => {
      pendingRequests.push({
        url,
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

it('forwards the selected model and effort when starting a session', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url === '/api/model-options?engine=claude') {
      return Promise.resolve(
        new Response(JSON.stringify(exampleModelOptions), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }
    if (url.startsWith('/api/skill-suggestions')) {
      return Promise.resolve(
        new Response(JSON.stringify({ skills: [] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }
    return Promise.resolve(
      new Response(JSON.stringify({ launchId: 'launch-fixture' }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  act(() => {
    currentState?.chooseDirectory({
      path: '/fixture/project',
      source: 'recent',
    })
    currentState?.chooseModel('fixture-model-orbit')
    currentState?.setSelectedEffort('high')
  })
  await act(async () => {
    await currentState?.start(() => undefined)
  })

  const launchRequest = fetcher.mock.calls.find(
    ([input]) => requestUrl(input) === '/api/sessions',
  )
  expect(
    launchRequest === undefined
      ? []
      : [
          {
            url: requestUrl(launchRequest[0]),
            method: launchRequest[1]?.method,
            headers: launchRequest[1]?.headers,
            body: launchRequest[1]?.body,
          },
        ],
  ).toEqual([
    {
      url: '/api/sessions',
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        directory: '/fixture/project',
        engine: 'claude',
        worktree: false,
        model: 'fixture-model-orbit',
        reasoningEffort: 'high',
      }),
    },
  ])
})

it('falls back to the new engine default and keeps a supported effort', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const codexOptions = {
    defaultModel: 'fixture-model-codex',
    defaultEffort: 'medium',
    models: [
      {
        id: 'fixture-model-codex',
        label: 'Codex fixture',
        defaultEffort: 'medium',
        efforts: ['medium', 'xhigh'],
      },
    ],
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    const options =
      url === '/api/model-options?engine=codex'
        ? codexOptions
        : exampleModelOptions
    return Promise.resolve(
      new Response(
        JSON.stringify(
          url.startsWith('/api/skill-suggestions') ? { skills: [] } : options,
        ),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  act(() => {
    currentState?.chooseModel('fixture-model-comet')
    currentState?.setSelectedEffort('xhigh')
    currentState?.setEngine('codex')
  })
  await flushRequests()

  const snapshot = () => ({
    engine: currentState?.engine,
    selectedModel: currentState?.selectedModel,
    selectedEffort: currentState?.selectedEffort,
    notice: currentState?.modelChangeNotice,
  })
  expect(snapshot()).toEqual({
    engine: 'codex',
    selectedModel: null,
    selectedEffort: 'xhigh',
    notice:
      'Codex has no fixture-model-comet, so the model is back to the default. Effort stays xhigh.',
  })
})

it('resets the model and effort and shows an error when options cannot load', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url === '/api/model-options?engine=codex') {
      return Promise.resolve(
        new Response(JSON.stringify({ error: 'Fixture catalog unavailable' }), {
          status: 503,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }
    const body = url.startsWith('/api/skill-suggestions')
      ? { skills: [] }
      : exampleModelOptions
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  act(() => {
    currentState?.chooseModel('fixture-model-comet')
    currentState?.setSelectedEffort('high')
    currentState?.setEngine('codex')
  })
  await flushRequests()

  const snapshot = () => ({
    engine: currentState?.engine,
    isLoadingModelOptions: currentState?.isLoadingModelOptions,
    modelOptions: currentState?.modelOptions,
    modelOptionsError: currentState?.modelOptionsError,
    selectedModel: currentState?.selectedModel,
    selectedEffort: currentState?.selectedEffort,
    notice: currentState?.modelChangeNotice,
  })
  expect(snapshot()).toEqual({
    engine: 'codex',
    isLoadingModelOptions: false,
    modelOptions: null,
    modelOptionsError: 'Fixture catalog unavailable',
    selectedModel: null,
    selectedEffort: null,
    notice:
      'Could not load Codex models, so the model and effort are back to the default.',
  })
})

it('undoes an engine change and restores the previous model and effort', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const codexOptions = {
    defaultModel: 'fixture-model-codex',
    defaultEffort: 'medium',
    models: [
      {
        id: 'fixture-model-codex',
        label: 'Codex fixture',
        defaultEffort: 'medium',
        efforts: ['medium'],
      },
    ],
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    const body =
      url === '/api/model-options?engine=codex'
        ? codexOptions
        : url.startsWith('/api/skill-suggestions')
          ? { skills: [] }
          : exampleModelOptions
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  act(() => {
    currentState?.chooseModel('fixture-model-comet')
    currentState?.setSelectedEffort('high')
    currentState?.setEngine('codex')
  })
  await flushRequests()
  act(() => currentState?.undoEngineChange())
  await flushRequests()

  const snapshot = () => ({
    engine: currentState?.engine,
    selectedModel: currentState?.selectedModel,
    selectedEffort: currentState?.selectedEffort,
    notice: currentState?.modelChangeNotice,
  })
  expect(snapshot()).toEqual({
    engine: 'claude',
    selectedModel: 'fixture-model-comet',
    selectedEffort: 'high',
    notice: null,
  })
})
