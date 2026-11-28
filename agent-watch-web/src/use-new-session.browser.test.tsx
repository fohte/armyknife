import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

import type { LaunchSessionRequest } from '#new-session-api'
import {
  decodeNewSessionDraft,
  NEW_SESSION_DRAFT_STORAGE_KEY,
} from '#new-session-draft'
import { exampleModelOptions } from '#new-session-story-fixtures.test'
import { type NewSessionState, useNewSession } from '#use-new-session'

let container: HTMLDivElement
let root: Root | null = null
let currentState: NewSessionState | null = null

function NewSessionHarness({ active = true }: { active?: boolean } = {}) {
  currentState = useNewSession(active)
  return null
}

function currentEngineSelection() {
  return {
    engine: currentState?.engine,
    selectedModel: currentState?.selectedModel,
    selectedEffort: currentState?.selectedEffort,
  }
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

async function flushDirectoryRequest() {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 200))
  })
}

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  currentState = null
  window.localStorage.removeItem(NEW_SESSION_DRAFT_STORAGE_KEY)
  container.remove()
  vi.unstubAllGlobals()
})

it('restores a saved draft and clears it back to defaults', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const savedAt = '2026-05-06T12:03:00.000Z'
  window.localStorage.setItem(
    NEW_SESSION_DRAFT_STORAGE_KEY,
    JSON.stringify({
      version: 1,
      savedAt,
      directory: { path: '/example/project', source: 'recent' },
      directoryQuery: 'project',
      engine: 'claude',
      selections: {
        claude: { model: 'fixture-model-orbit', effort: 'high' },
        codex: { model: null, effort: null },
      },
      worktree: true,
      initialMessage: 'Continue the setup.',
    }),
  )
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>((input) => {
      const url = requestUrl(input)
      const body = url.startsWith('/api/model-options')
        ? exampleModelOptions
        : { skills: [] }
      return Promise.resolve(
        new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }),
  )
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  const restored = () => ({
    directory: currentState?.selectedDirectory,
    directoryQuery: currentState?.directoryQuery,
    engine: currentState?.engine,
    model: currentState?.selectedModel,
    effort: currentState?.selectedEffort,
    worktree: currentState?.worktree,
    initialMessage: currentState?.initialMessage,
    restoredAt: currentState?.restoredAt,
  })
  expect(restored()).toEqual({
    directory: { path: '/example/project', source: 'recent' },
    directoryQuery: 'project',
    engine: 'claude',
    model: 'fixture-model-orbit',
    effort: 'high',
    worktree: true,
    initialMessage: 'Continue the setup.',
    restoredAt: savedAt,
  })

  act(() => currentState?.clearDraft())

  const cleared = () => ({
    directory: currentState?.selectedDirectory,
    directoryQuery: currentState?.directoryQuery,
    engine: currentState?.engine,
    worktree: currentState?.worktree,
    initialMessage: currentState?.initialMessage,
    restoredAt: currentState?.restoredAt,
    modelOptions: currentState?.modelOptions,
    storedDraft: window.localStorage.getItem(NEW_SESSION_DRAFT_STORAGE_KEY),
  })
  expect(cleared()).toEqual({
    directory: null,
    directoryQuery: '',
    engine: 'claude',
    worktree: false,
    initialMessage: '',
    restoredAt: null,
    modelOptions: exampleModelOptions,
    storedDraft: null,
  })
})

it('keeps the saved time when unavailable model choices are reconciled', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const savedDraft = JSON.stringify({
    version: 1,
    savedAt: '2026-05-06T12:03:00.000Z',
    directory: null,
    directoryQuery: '',
    engine: 'claude',
    selections: {
      claude: { model: 'retired-fixture-model', effort: 'xhigh' },
      codex: { model: null, effort: null },
    },
    worktree: false,
    initialMessage: '',
  })
  window.localStorage.setItem(NEW_SESSION_DRAFT_STORAGE_KEY, savedDraft)
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>((input) => {
      const url = requestUrl(input)
      const body = url.startsWith('/api/model-options')
        ? exampleModelOptions
        : { skills: [] }
      return Promise.resolve(
        new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }),
  )
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()

  const snapshot = () => ({
    selectedModel: currentState?.selectedModel,
    selectedEffort: currentState?.selectedEffort,
    storedDraft: window.localStorage.getItem(NEW_SESSION_DRAFT_STORAGE_KEY),
  })
  expect(snapshot()).toEqual({
    selectedModel: null,
    selectedEffort: null,
    storedDraft: savedDraft,
  })
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
      '/api/model-options?engine=codex',
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

it('passes the selected model and effort to the launch handler', async () => {
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
  const requests: unknown[] = []
  act(() => {
    currentState?.start((request) => requests.push(request))
  })
  expect(requests).toEqual([
    {
      directory: '/fixture/project',
      engine: 'claude',
      worktree: false,
      model: 'fixture-model-orbit',
      reasoningEffort: 'high',
      initialMessage: '',
    },
  ])
})

it('uses profile defaults after clearing an effort unsupported by the default model', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const modelOptions = {
    defaultModel: 'fixture-profile-model',
    defaultEffort: 'medium',
    models: [
      {
        id: 'fixture-model-orbit',
        label: 'Orbit',
        defaultEffort: 'low',
        efforts: ['low', 'medium'],
      },
      {
        id: 'fixture-model-comet',
        label: 'Comet',
        defaultEffort: 'high',
        efforts: ['medium', 'high', 'xhigh'],
      },
    ],
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    const body =
      url === '/api/model-options?engine=claude'
        ? modelOptions
        : url.startsWith('/api/skill-suggestions')
          ? { skills: [] }
          : url.startsWith('/api/directories')
            ? { directories: [] }
            : { launchId: 'launch-fixture' }
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
  await flushDirectoryRequest()
  act(() => {
    currentState?.chooseDirectory({
      path: '/fixture/project',
      source: 'recent',
    })
    currentState?.chooseModel('fixture-model-comet')
    currentState?.setSelectedEffort('xhigh')
    currentState?.chooseModel(null)
  })
  const requests: unknown[] = []
  act(() => {
    currentState?.start((request) => requests.push(request))
  })
  expect(requests).toEqual([
    {
      directory: '/fixture/project',
      engine: 'claude',
      worktree: false,
      initialMessage: '',
    },
  ])
})

const codexModelOptions = {
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

it('loads and reconciles model selections for both engines on activation', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const savedDraft = JSON.stringify({
    version: 1,
    savedAt: '2026-05-06T12:03:00.000Z',
    directory: null,
    directoryQuery: '',
    engine: 'claude',
    selections: {
      claude: { model: 'retired-fixture-model', effort: 'xhigh' },
      codex: { model: 'fixture-model-codex', effort: 'xhigh' },
    },
    worktree: false,
    initialMessage: '',
  })
  window.localStorage.setItem(NEW_SESSION_DRAFT_STORAGE_KEY, savedDraft)
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>((input) => {
      const url = new URL(requestUrl(input), window.location.href)
      const body =
        url.pathname === '/api/model-options'
          ? url.searchParams.get('engine') === 'codex'
            ? codexModelOptions
            : exampleModelOptions
          : { skills: [] }
      return Promise.resolve(
        new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }),
  )
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()

  const snapshot = () => ({
    engineOptions: currentState?.engineOptions,
    storedDraft: window.localStorage.getItem(NEW_SESSION_DRAFT_STORAGE_KEY),
  })
  expect(snapshot()).toEqual({
    engineOptions: {
      claude: {
        modelOptions: exampleModelOptions,
        isLoading: false,
        error: null,
        selectedModel: null,
        selectedEffort: null,
        activeModelOption: exampleModelOptions.models[0],
      },
      codex: {
        modelOptions: codexModelOptions,
        isLoading: false,
        error: null,
        selectedModel: 'fixture-model-codex',
        selectedEffort: 'xhigh',
        activeModelOption: codexModelOptions.models[0],
      },
    },
    storedDraft: savedDraft,
  })
})

it('changes Codex model and effort without changing Claude and stores the draft', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>((input) => {
      const url = new URL(requestUrl(input), window.location.href)
      const body =
        url.pathname === '/api/model-options'
          ? url.searchParams.get('engine') === 'codex'
            ? codexModelOptions
            : exampleModelOptions
          : { skills: [] }
      return Promise.resolve(
        new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }),
  )
  root = createRoot(container)

  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  act(() => {
    currentState?.chooseModelFor('codex', 'fixture-model-codex')
    currentState?.chooseEffortFor('codex', 'xhigh')
  })

  const snapshot = () => {
    const rawDraft = window.localStorage.getItem(NEW_SESSION_DRAFT_STORAGE_KEY)
    const storedDraft =
      rawDraft === null ? null : decodeNewSessionDraft(rawDraft)
    if (storedDraft !== null) {
      storedDraft.savedAt = '<normalized timestamp>'
    }
    return {
      engineOptions: currentState?.engineOptions,
      storedDraft,
    }
  }
  expect(snapshot()).toEqual({
    engineOptions: {
      claude: {
        modelOptions: exampleModelOptions,
        isLoading: false,
        error: null,
        selectedModel: null,
        selectedEffort: null,
        activeModelOption: exampleModelOptions.models[0],
      },
      codex: {
        modelOptions: codexModelOptions,
        isLoading: false,
        error: null,
        selectedModel: 'fixture-model-codex',
        selectedEffort: 'xhigh',
        activeModelOption: codexModelOptions.models[0],
      },
    },
    storedDraft: {
      version: 1,
      savedAt: '<normalized timestamp>',
      directory: null,
      directoryQuery: '',
      engine: 'claude',
      selections: {
        claude: { model: null, effort: null },
        codex: { model: 'fixture-model-codex', effort: 'xhigh' },
      },
      worktree: false,
      initialMessage: '',
    },
  })
})

it('restores each engine model and effort selection after switching', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    const body =
      url === '/api/model-options?engine=codex'
        ? codexModelOptions
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
  const codexStartsEmpty = currentEngineSelection()
  act(() => {
    currentState?.chooseModel('fixture-model-codex')
    currentState?.setSelectedEffort('xhigh')
    currentState?.setEngine('claude')
  })
  await flushRequests()
  const claudeSelectionRestored = currentEngineSelection()
  act(() => currentState?.setEngine('codex'))
  await flushRequests()

  const selectionHistory = () => ({
    codexStartsEmpty,
    claudeSelectionRestored,
    codexSelectionRestored: currentEngineSelection(),
  })
  expect(selectionHistory()).toEqual({
    codexStartsEmpty: {
      engine: 'codex',
      selectedModel: null,
      selectedEffort: null,
    },
    claudeSelectionRestored: {
      engine: 'claude',
      selectedModel: 'fixture-model-comet',
      selectedEffort: 'high',
    },
    codexSelectionRestored: {
      engine: 'codex',
      selectedModel: 'fixture-model-codex',
      selectedEffort: 'xhigh',
    },
  })
})

it('passes the overridden engine model and effort selection to launch', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    const body =
      url === '/api/model-options?engine=codex'
        ? codexModelOptions
        : url.startsWith('/api/skill-suggestions')
          ? { skills: [] }
          : url === '/api/sessions'
            ? { launchId: 'launch-fixture' }
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
    currentState?.chooseDirectory({
      path: '/fixture/project',
      source: 'recent',
    })
    currentState?.chooseModel('fixture-model-comet')
    currentState?.setSelectedEffort('high')
    currentState?.setEngine('codex')
  })
  await flushRequests()
  act(() => {
    currentState?.chooseModel('fixture-model-codex')
    currentState?.setSelectedEffort('xhigh')
    currentState?.setEngine('claude')
  })
  await flushRequests()
  const launchRequests: LaunchSessionRequest[] = []
  act(() => {
    currentState?.start((request) => launchRequests.push(request), 'codex')
  })

  expect(launchRequests).toEqual([
    {
      directory: '/fixture/project',
      engine: 'codex',
      worktree: false,
      model: 'fixture-model-codex',
      reasoningEffort: 'xhigh',
      initialMessage: '',
    },
  ])
})

it('retries a failed engine catalog and restores its selection', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  let codexLoadCount = 0
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url === '/api/model-options?engine=codex') {
      codexLoadCount += 1
      if (codexLoadCount < 3) {
        return Promise.resolve(
          new Response(
            JSON.stringify({ error: 'Fixture catalog unavailable' }),
            {
              status: 503,
              headers: { 'content-type': 'application/json' },
            },
          ),
        )
      }
    }
    const body =
      url === '/api/model-options?engine=codex'
        ? codexModelOptions
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
  act(() => currentState?.setEngine('codex'))
  await flushRequests()
  act(() => {
    currentState?.chooseModel('fixture-model-codex')
    currentState?.setSelectedEffort('xhigh')
    currentState?.setEngine('claude')
  })
  await flushRequests()
  const retryFailure = currentState?.engineOptions.codex
  act(() => currentState?.setEngine('codex'))
  await flushRequests()

  const snapshot = () => ({
    engineOptions: currentState?.engineOptions,
    retryFailure,
    modelOptionRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.startsWith('/api/model-options')),
    currentEngineState: {
      engine: currentState?.engine,
      isLoadingModelOptions: currentState?.isLoadingModelOptions,
      modelOptions: currentState?.modelOptions,
      modelOptionsError: currentState?.modelOptionsError,
      selectedModel: currentState?.selectedModel,
      selectedEffort: currentState?.selectedEffort,
    },
  })
  expect(snapshot()).toEqual({
    engineOptions: {
      claude: {
        modelOptions: exampleModelOptions,
        isLoading: false,
        error: null,
        selectedModel: null,
        selectedEffort: null,
        activeModelOption: exampleModelOptions.models[0],
      },
      codex: {
        modelOptions: codexModelOptions,
        isLoading: false,
        error: null,
        selectedModel: 'fixture-model-codex',
        selectedEffort: 'xhigh',
        activeModelOption: codexModelOptions.models[0],
      },
    },
    retryFailure: {
      modelOptions: null,
      isLoading: false,
      error: 'Fixture catalog unavailable',
      selectedModel: 'fixture-model-codex',
      selectedEffort: 'xhigh',
      activeModelOption: undefined,
    },
    modelOptionRequests: [
      '/api/model-options?engine=claude',
      '/api/model-options?engine=codex',
      '/api/model-options?engine=codex',
      '/api/model-options?engine=codex',
    ],
    currentEngineState: {
      engine: 'codex',
      isLoadingModelOptions: false,
      modelOptions: codexModelOptions,
      modelOptionsError: null,
      selectedModel: 'fixture-model-codex',
      selectedEffort: 'xhigh',
    },
  })
})

it('ignores a retry response from an earlier activation', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  let codexLoadCount = 0
  let resolveRetry: ((response: Response) => void) | undefined
  const staleModelOptions = {
    defaultModel: 'stale-fixture-model',
    defaultEffort: 'low',
    models: [
      {
        id: 'stale-fixture-model',
        label: 'Stale fixture',
        defaultEffort: 'low',
        efforts: ['low'],
      },
    ],
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url === '/api/model-options?engine=codex') {
      codexLoadCount += 1
      if (codexLoadCount === 1) {
        return Promise.resolve(
          new Response(
            JSON.stringify({ error: 'Fixture catalog unavailable' }),
            {
              status: 503,
              headers: { 'content-type': 'application/json' },
            },
          ),
        )
      }
      if (codexLoadCount === 2) {
        return new Promise((resolve) => {
          resolveRetry = resolve
        })
      }
      return Promise.resolve(
        new Response(JSON.stringify(codexModelOptions), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }
    const body =
      url === '/api/model-options?engine=claude'
        ? exampleModelOptions
        : { skills: [] }
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
  act(() => currentState?.setEngine('codex'))
  await flushRequests()
  act(() => root?.render(<NewSessionHarness active={false} />))
  act(() => root?.render(<NewSessionHarness />))
  await flushRequests()
  act(() => currentState?.chooseModel('fixture-model-codex'))

  if (resolveRetry === undefined) {
    throw new Error('Expected a pending Codex retry')
  }
  await act(async () => {
    resolveRetry(
      new Response(JSON.stringify(staleModelOptions), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    )
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    codex: currentState?.engineOptions.codex,
    modelOptionRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.startsWith('/api/model-options')),
  })
  expect(snapshot()).toEqual({
    codex: {
      modelOptions: codexModelOptions,
      isLoading: false,
      error: null,
      selectedModel: 'fixture-model-codex',
      selectedEffort: null,
      activeModelOption: codexModelOptions.models[0],
    },
    modelOptionRequests: [
      '/api/model-options?engine=claude',
      '/api/model-options?engine=codex',
      '/api/model-options?engine=codex',
      '/api/model-options?engine=claude',
      '/api/model-options?engine=codex',
    ],
  })
})

it('clears model and effort selections for both engines on reset', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    const body =
      url === '/api/model-options?engine=codex'
        ? codexModelOptions
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
  act(() => {
    currentState?.chooseModel('fixture-model-codex')
    currentState?.setSelectedEffort('xhigh')
    currentState?.reset()
  })
  await flushRequests()
  const claudeSelection = currentEngineSelection()
  act(() => currentState?.setEngine('codex'))
  await flushRequests()

  const selectionHistory = () => ({
    claudeSelection,
    codexSelection: currentEngineSelection(),
  })
  expect(selectionHistory()).toEqual({
    claudeSelection: {
      engine: 'claude',
      selectedModel: null,
      selectedEffort: null,
    },
    codexSelection: {
      engine: 'codex',
      selectedModel: null,
      selectedEffort: null,
    },
  })
})
