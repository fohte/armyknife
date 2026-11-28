import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

import type { AsyncAction } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'

let container: HTMLDivElement
let root: Root | null = null

const failedCloseAction: AsyncAction = {
  id: 1,
  kind: 'close',
  discardable: true,
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

it('keeps discard and close failure controls aligned on desktop and phone', async () => {
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
        onDiscardChanges={() => undefined}
        onOpenInTerminal={() => undefined}
      />,
    )
  })

  const desktop = () => {
    const options = container.querySelector<HTMLElement>(
      '.async-action-controls--close-options',
    )
    const secondary = container.querySelector<HTMLElement>(
      '.async-action-controls--close-secondary',
    )
    const buttons = Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.async-action-controls--close-options button, .async-action-controls--close-secondary button',
      ),
    )
    const [keepButton, discardButton, retryButton, terminalButton] = buttons
    if (
      options === null ||
      secondary === null ||
      buttons.length !== 4 ||
      keepButton === undefined ||
      discardButton === undefined ||
      retryButton === undefined ||
      terminalButton === undefined
    ) {
      throw new Error('The close failure controls are missing')
    }
    const keep = keepButton.getBoundingClientRect()
    const discard = discardButton.getBoundingClientRect()
    const retry = retryButton.getBoundingClientRect()
    const terminal = terminalButton.getBoundingClientRect()
    return {
      discardSharesRowWithKeep: Math.abs(keep.y - discard.y) < 1,
      retrySharesRowWithDetails: Math.abs(retry.y - terminal.y) < 1,
      buttonsUseTextWidths: [keep, discard, retry, terminal].every(
        (rect) =>
          rect.width <
          (rect === keep || rect === discard
            ? options.getBoundingClientRect().width
            : secondary.getBoundingClientRect().width),
      ),
      bothGroupsAlignLeft:
        Math.abs(keep.x - options.getBoundingClientRect().x) < 1 &&
        Math.abs(retry.x - secondary.getBoundingClientRect().x) < 1,
    }
  }

  const desktopLayout = desktop()
  await page.viewport(390, 844)
  container.style.width = '390px'
  const phoneLayout = () => {
    const options = container.querySelector<HTMLElement>(
      '.async-action-controls--close-options',
    )
    const secondary = container.querySelector<HTMLElement>(
      '.async-action-controls--close-secondary',
    )
    const buttons = Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.async-action-controls--close-options button, .async-action-controls--close-secondary button',
      ),
    )
    const [keepButton, discardButton, retryButton, terminalButton] = buttons
    if (
      options === null ||
      secondary === null ||
      buttons.length !== 4 ||
      keepButton === undefined ||
      discardButton === undefined ||
      retryButton === undefined ||
      terminalButton === undefined
    ) {
      throw new Error('The phone close failure controls are missing')
    }
    const keep = keepButton.getBoundingClientRect()
    const discard = discardButton.getBoundingClientRect()
    const retry = retryButton.getBoundingClientRect()
    const terminal = terminalButton.getBoundingClientRect()
    return {
      discardAndKeepShareRow: Math.abs(keep.y - discard.y) < 1,
      retryAndTerminalShareRow: Math.abs(retry.y - terminal.y) < 1,
      secondaryIsBelowOptions: retry.y > keep.bottom,
      buttonsAre44pxTall: [keep, discard, retry, terminal].every(
        (rect) => rect.height === 44,
      ),
    }
  }

  const snapshot = () => ({ desktop: desktopLayout, phone: phoneLayout() })
  const expected = {
    desktop: {
      discardSharesRowWithKeep: true,
      retrySharesRowWithDetails: true,
      buttonsUseTextWidths: true,
      bothGroupsAlignLeft: true,
    },
    phone: {
      discardAndKeepShareRow: true,
      retryAndTerminalShareRow: true,
      secondaryIsBelowOptions: true,
      buttonsAre44pxTall: true,
    },
  }
  expect(snapshot()).toEqual(expected)
})

it('opens the discard confirmation from a close failure', async () => {
  const onDiscardChanges = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionItem
        action={failedCloseAction}
        form="toast"
        onDiscardChanges={onDiscardChanges}
      />,
    )
  })

  const discardButton = Array.from(container.querySelectorAll('button')).find(
    (button) => button.textContent === 'Discard changes…',
  )
  if (discardButton === undefined) {
    throw new Error('Expected the discard action')
  }
  const keepButton = Array.from(container.querySelectorAll('button')).find(
    (button) => button.textContent === 'Keep worktree',
  )
  const detailsButton = Array.from(container.querySelectorAll('button')).find(
    (button) => button.textContent === 'Details',
  )
  await userEvent.click(discardButton)

  const snapshot = () => ({
    keepText: keepButton?.textContent ?? null,
    keepLabel: keepButton?.getAttribute('aria-label') ?? null,
    discardLabel: discardButton.getAttribute('aria-label'),
    detailsLabel: detailsButton?.getAttribute('aria-label') ?? null,
    calls: onDiscardChanges.mock.calls,
  })
  expect(snapshot()).toEqual({
    keepText: 'Keep worktree',
    keepLabel: 'Close and keep worktree',
    discardLabel: 'Discard changes and close…',
    detailsLabel: 'Details in Activity',
    calls: [[]],
  })
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
