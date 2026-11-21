import { ok } from 'neverthrow'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { useCommandPaletteController } from '#command-palette-controller'
import type { SessionOperation } from '#session-operation-types'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null
let controller: ReturnType<typeof useCommandPaletteController> | null = null

type ControllerOptions = Parameters<typeof useCommandPaletteController>[0]

type HarnessProps = {
  sessions: readonly Session[]
  runOperation: ControllerOptions['runOperation']
}

function ControllerHarness({ sessions, runOperation }: HarnessProps) {
  controller = useCommandPaletteController({
    sessions,
    requestComposerFocus: () => undefined,
    navigate: () => undefined,
    setError: () => undefined,
    setFocusedProjectName: () => undefined,
    setLaterExpanded: () => undefined,
    selectedId: null,
    selectedSessionExists: false,
    view: 'inbox',
    isMobile: false,
    conversationOpen: false,
    runOperation,
  })
  return null
}

function session(
  id: string,
  category: Session['category'],
  todoLaterCount = 0,
): Session {
  return {
    id,
    category,
    isPaused: false,
    kind: null,
    title: id,
    preview: null,
    elapsedSeconds: 0,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'sample-project',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
    todoLaterCount,
  }
}

function currentController() {
  if (controller === null) throw new Error('Expected a mounted controller')
  return controller
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  root = null
  controller = null
  container.remove()
  vi.unstubAllGlobals()
})

it('skips done sessions that still have folded to-dos during bulk close', async () => {
  const operations: SessionOperation[] = []
  const runOperation: HarnessProps['runOperation'] = (operation) => {
    operations.push(operation)
    return Promise.resolve(ok(null))
  }
  root = createRoot(container)
  act(() => {
    root?.render(
      <ControllerHarness
        sessions={[
          session('finished-without-later-todos', 'done'),
          session('finished-with-folded-todos', 'done', 2),
          session('still-running', 'in_progress'),
        ]}
        runOperation={runOperation}
      />,
    )
  })

  await act(async () => {
    await currentController().closeDoneSessions([
      session('finished-without-later-todos', 'done'),
      session('finished-with-folded-todos', 'done', 2),
      session('still-running', 'in_progress'),
    ])
  })

  expect(operations).toEqual([
    {
      type: 'close',
      sessionId: 'finished-without-later-todos',
      keepWorktree: false,
    },
  ])
})
