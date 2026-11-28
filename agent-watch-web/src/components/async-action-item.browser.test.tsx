import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import type { AsyncAction } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'

let container: HTMLDivElement
let root: Root | null = null

const failedCloseAction: AsyncAction = {
  id: 1,
  kind: 'close',
  sessionId: 'example-close-session',
  sessionTitle: 'Review the notification route',
  status: 'closeFailed',
  error: 'The example workspace could not be closed.',
  checks: {
    path: '~/work/example-service',
    files: [],
    changes: [],
    commits: [],
    branch: 'feature/example-route',
    merged: true,
    reason: null,
    processes: [
      { pid: 23451, name: 'example server' },
      { pid: 23452, name: 'example worker' },
    ],
    commitInspectionUnavailable: false,
    processInspectionUnavailable: false,
  },
  createdAt: 1_799_000_000_000,
}

const failedMergeAction: AsyncAction = {
  id: 2,
  kind: 'merge',
  sessionId: 'example-merge-session',
  sessionTitle: 'Review the rollout monitor',
  pullRequestNumber: 48,
  status: 'mergeFailed',
  error:
    'The example branch changed after the latest review. Confirm the new version before merging.',
  createdAt: 1_799_000_000_003,
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('keeps close failure controls compact on desktop and unchanged on phone', async () => {
  await page.viewport(1440, 900)
  container.style.width = '400px'
  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionItem
        action={failedCloseAction}
        form="row"
        onRetry={() => undefined}
        onCloseKeepWorktree={() => undefined}
        onOpenInTerminal={() => undefined}
      />,
    )
  })

  const desktop = () => {
    const controls = container.querySelector<HTMLElement>(
      '.async-action-controls',
    )
    const buttons = Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.async-action-controls button',
      ),
    )
    const [keepButton, retryButton, terminalButton] = buttons
    if (
      controls === null ||
      buttons.length !== 3 ||
      keepButton === undefined ||
      retryButton === undefined ||
      terminalButton === undefined
    ) {
      throw new Error('The close failure controls are missing')
    }
    const controlsRect = controls.getBoundingClientRect()
    const keep = keepButton.getBoundingClientRect()
    const retry = retryButton.getBoundingClientRect()
    const terminal = terminalButton.getBoundingClientRect()
    return {
      keepAndRetryShareRow: Math.abs(keep.y - retry.y) < 1,
      terminalIsBelow: terminal.y > retry.bottom,
      buttonsUseTextWidths: [keep, retry, terminal].every(
        (rect) => rect.width < controlsRect.width,
      ),
      buttonsAlignLeft:
        Math.abs(keep.x - controlsRect.x) < 1 &&
        Math.abs(terminal.x - controlsRect.x) < 1,
    }
  }

  const desktopLayout = desktop()
  await page.viewport(390, 844)
  container.style.width = '390px'
  const phoneLayout = () => {
    const controls = container.querySelector<HTMLElement>(
      '.async-action-controls',
    )
    const buttons = Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.async-action-controls button',
      ),
    )
    const [keepButton, retryButton, terminalButton] = buttons
    if (
      controls === null ||
      buttons.length !== 3 ||
      keepButton === undefined ||
      retryButton === undefined ||
      terminalButton === undefined
    ) {
      throw new Error('The phone close failure controls are missing')
    }
    const controlsRect = controls.getBoundingClientRect()
    const keep = keepButton.getBoundingClientRect()
    const retry = retryButton.getBoundingClientRect()
    const terminal = terminalButton.getBoundingClientRect()
    return {
      keepUsesFullRow: Math.abs(keep.width - controlsRect.width) < 1,
      retryAndTerminalShareRow: Math.abs(retry.y - terminal.y) < 1,
      keepIsAboveActions: retry.y > keep.bottom,
      buttonsAre44pxTall: [keep, retry, terminal].every(
        (rect) => rect.height === 44,
      ),
    }
  }

  const snapshot = () => ({ desktop: desktopLayout, phone: phoneLayout() })
  const expected = {
    desktop: {
      keepAndRetryShareRow: true,
      terminalIsBelow: true,
      buttonsUseTextWidths: true,
      buttonsAlignLeft: true,
    },
    phone: {
      keepUsesFullRow: true,
      retryAndTerminalShareRow: true,
      keepIsAboveActions: true,
      buttonsAre44pxTall: true,
    },
  }
  expect(snapshot()).toEqual(expected)
})

it('shows merge failure detail and Retry as the desktop design does', async () => {
  await page.viewport(1440, 900)
  container.style.width = '400px'
  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionItem
        action={failedMergeAction}
        form="row"
        onRetry={() => undefined}
      />,
    )
  })

  const snapshot = () => {
    const controls = container.querySelector<HTMLElement>(
      '.async-action-controls',
    )
    const retry = container.querySelector<HTMLButtonElement>(
      '.async-action-controls button',
    )
    const error = container.querySelector<HTMLElement>('.async-action-error')
    if (controls === null || retry === null || error === null) {
      throw new Error('The merge failure detail is missing')
    }
    const controlsRect = controls.getBoundingClientRect()
    const retryRect = retry.getBoundingClientRect()
    const lines = document.createRange()
    lines.selectNodeContents(error)
    return {
      errorColor: getComputedStyle(error).color,
      errorLines: lines.getClientRects().length,
      retryUsesTextWidth: retryRect.width < controlsRect.width,
      retryAlignsLeft: Math.abs(retryRect.x - controlsRect.x) < 1,
      retryIs88px: Math.abs(retryRect.width - 88) < 1,
      retryIs44pxTall: retryRect.height === 44,
    }
  }

  const desktop = snapshot()
  await page.viewport(390, 844)
  container.style.width = '390px'
  const phone = snapshot()

  const combinedSnapshot = () => ({ desktop, phone })
  const expected = {
    desktop: {
      errorColor: 'rgb(212, 212, 216)',
      errorLines: 2,
      retryUsesTextWidth: true,
      retryAlignsLeft: true,
      retryIs88px: false,
      retryIs44pxTall: false,
    },
    phone: {
      errorColor: 'rgb(212, 212, 216)',
      errorLines: 2,
      retryUsesTextWidth: true,
      retryAlignsLeft: true,
      retryIs88px: true,
      retryIs44pxTall: true,
    },
  }
  expect(combinedSnapshot()).toEqual(expected)
})

it('hides retry for PR actions that require a new review', () => {
  root = createRoot(container)
  const action = {
    id: 3,
    kind: 'review_document',
    sessionId: 'example-review-session',
    sessionTitle: 'Review the notification route',
    status: 'reviewDocumentActionFailed',
    reviewDocumentAction: 'Update PR #48',
    error:
      'The pull request target changed after approval. Send the draft back for review before submitting again.',
    retryable: false,
    createdAt: 1_799_000_000_006,
  } satisfies AsyncAction

  act(() => {
    root?.render(
      <AsyncActionItem
        action={action}
        form="toast"
        onRetry={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    buttons: Array.from(
      container.querySelectorAll('.async-action-controls button'),
    ).map((button) => button.textContent),
    error: container.querySelector('.async-action-error')?.textContent ?? null,
  })
  const expected = {
    buttons: [],
    error:
      'The pull request target changed after approval. Send the draft back for review before submitting again.',
  }
  expect(snapshot()).toEqual(expected)
})

it.each([
  {
    name: 'engine_and_repo',
    engine: 'claude' as const,
    repo: '/example/service',
    expected: 'Starting Claude Code in /example/service…',
  },
  {
    name: 'missing_engine',
    repo: '/example/service',
    expected: 'Starting session in /example/service…',
  },
  {
    name: 'missing_repo',
    engine: 'codex' as const,
    expected: 'Starting Codex…',
  },
])('formats the launch line with $name', ({ engine, repo, expected }) => {
  const action: AsyncAction = {
    id: 4,
    kind: 'launch',
    sessionId: 'example-launch-session',
    sessionTitle: 'Review the example request',
    status: 'launching',
    ...(engine === undefined ? {} : { engine }),
    ...(repo === undefined ? {} : { repo }),
    createdAt: 1_799_000_000_009,
  }
  root = createRoot(container)
  act(() => {
    root?.render(<AsyncActionItem action={action} form="line" />)
  })

  const snapshot = () => ({
    role: container.querySelector('[role="status"]')?.getAttribute('role'),
    line: container.querySelector('.async-action-item-content strong')
      ?.textContent,
  })
  expect(snapshot()).toEqual({ role: 'status', line: expected })
})
