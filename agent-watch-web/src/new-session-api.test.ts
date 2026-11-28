import { afterEach, expect, it, vi } from 'vitest'

import {
  getSessionDirectories,
  getSessionModelOptions,
  getSessionSkillSuggestions,
  launchSession,
} from '#new-session-api'

afterEach(() => {
  vi.unstubAllGlobals()
})

it('searches directory suggestions with the encoded query', async () => {
  const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(
      JSON.stringify({
        directories: [
          {
            path: '/Users/example/work/service',
            source: 'recent',
            lastUsedAt: '2026-05-06T12:00:00Z',
          },
          { path: '/Users/example/work/handbook', source: 'repo' },
        ],
      }),
      { status: 200, headers: { 'content-type': 'application/json' } },
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  const result = await getSessionDirectories('service docs')

  const snapshot = () => ({
    result: result.match(
      (value) => value,
      (error) => error.message,
    ),
    requests: fetcher.mock.calls,
  })
  expect(snapshot()).toEqual({
    result: [
      {
        path: '/Users/example/work/service',
        source: 'recent',
        lastUsedAt: '2026-05-06T12:00:00Z',
      },
      { path: '/Users/example/work/handbook', source: 'repo' },
    ],
    requests: [['/api/directories?q=service+docs']],
  })
})

it('loads skill suggestions for the selected engine and optional directory', async () => {
  const fetcher = vi.fn<typeof fetch>().mockImplementation(() =>
    Promise.resolve(
      new Response(JSON.stringify({ skills: [] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  const claudeResult = await getSessionSkillSuggestions(
    'claude',
    '/Users/example/work/feature branch',
  )
  const codexResult = await getSessionSkillSuggestions('codex', null)

  const snapshot = () => ({
    results: [claudeResult, codexResult].map((result) =>
      result.match(
        (value) => value,
        (error) => error.message,
      ),
    ),
    requests: fetcher.mock.calls,
  })
  expect(snapshot()).toEqual({
    results: [[], []],
    requests: [
      [
        '/api/skill-suggestions?engine=claude&directory=%2FUsers%2Fexample%2Fwork%2Ffeature+branch',
      ],
      ['/api/skill-suggestions?engine=codex'],
    ],
  })
})

it('loads the model catalog for the selected engine', async () => {
  const modelOptions = {
    defaultModel: 'fixture-model-orbit',
    defaultEffort: 'medium',
    models: [
      {
        id: 'fixture-model-orbit',
        label: 'Orbit',
        defaultEffort: 'medium',
        efforts: ['low', 'medium', 'high'],
      },
    ],
  }
  const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(JSON.stringify(modelOptions), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  )
  vi.stubGlobal('fetch', fetcher)

  const result = await getSessionModelOptions('codex')

  const snapshot = () => ({
    result: result.match(
      (value) => value,
      (error) => error.message,
    ),
    requests: fetcher.mock.calls.map(([url]) => url),
  })
  expect(snapshot()).toEqual({
    result: modelOptions,
    requests: ['/api/model-options?engine=codex'],
  })
})

it('launches the selected engine and omits a blank initial message', async () => {
  const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(JSON.stringify({ launchId: 'launch-fixture' }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  )
  vi.stubGlobal('fetch', fetcher)

  const result = await launchSession({
    directory: '/Users/example/work/service',
    engine: 'codex',
    worktree: true,
    initialMessage: '  ',
  })

  const snapshot = () => ({
    result: result.match(
      (value) => value,
      (error) => error.message,
    ),
    requests: fetcher.mock.calls,
  })
  expect(snapshot()).toEqual({
    result: { launchId: 'launch-fixture' },
    requests: [
      [
        '/api/sessions',
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            directory: '/Users/example/work/service',
            engine: 'codex',
            worktree: true,
          }),
        },
      ],
    ],
  })
})

it('includes the first message in the launch request', async () => {
  const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(JSON.stringify({ launchId: 'launch-fixture' }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  )
  vi.stubGlobal('fetch', fetcher)

  const result = await launchSession({
    directory: '/Users/example/work/service',
    engine: 'claude',
    worktree: false,
    initialMessage: 'Add the retention option.',
  })
  const snapshot = () => ({
    result: result.match(
      (value) => value,
      (error) => error.message,
    ),
    requests: fetcher.mock.calls,
  })

  expect(snapshot()).toEqual({
    result: { launchId: 'launch-fixture' },
    requests: [
      [
        '/api/sessions',
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            directory: '/Users/example/work/service',
            engine: 'claude',
            worktree: false,
            initialMessage: 'Add the retention option.',
          }),
        },
      ],
    ],
  })
})

it('includes selected model and effort in the launch request', async () => {
  const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(JSON.stringify({ launchId: 'launch-fixture' }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  )
  vi.stubGlobal('fetch', fetcher)

  await launchSession({
    directory: '/Users/example/work/service',
    engine: 'codex',
    worktree: false,
    model: 'fixture-model-orbit',
    reasoningEffort: 'xhigh',
  })

  expect(fetcher.mock.calls).toEqual([
    [
      '/api/sessions',
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          directory: '/Users/example/work/service',
          engine: 'codex',
          worktree: false,
          model: 'fixture-model-orbit',
          reasoningEffort: 'xhigh',
        }),
      },
    ],
  ])
})
