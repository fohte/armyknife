import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { WebUiPane } from '#components/web-ui-pane'

let container: HTMLDivElement
let root: Root | null = null

function relayTargets() {
  return {
    iframe: container
      .querySelector<HTMLIFrameElement>('iframe')
      ?.getAttribute('src'),
    link: container.querySelector<HTMLAnchorElement>('a')?.getAttribute('href'),
  }
}

function unavailablePaneState() {
  return {
    iframe: container.querySelector('iframe')?.getAttribute('src') ?? null,
    link: container.querySelector('a')?.getAttribute('href') ?? null,
    status: container.querySelector('[role="status"]')?.textContent,
    localUrl: container.querySelector(
      '.web-ui-pane-header span:not(.web-ui-pane-title)',
    )?.textContent,
  }
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
  vi.unstubAllGlobals()
})

it('toggles the review between its column and fullscreen layouts', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Editor"
        url="http://127.0.0.1:5000/review/sample"
        onClose={() => undefined}
      />,
    )
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Show Editor fullscreen"]',
      )
      ?.click()
  })
  const fullscreenState = {
    className: container.querySelector('aside')?.className,
    exitLabel: container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Exit Editor fullscreen"]',
      )
      ?.getAttribute('aria-pressed'),
  }

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Exit Editor fullscreen"]',
      )
      ?.click()
  })

  const snapshot = () => ({
    fullscreenState,
    restoredClassName: container.querySelector('aside')?.className,
    restoredLabel: container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Show Editor fullscreen"]',
      )
      ?.getAttribute('aria-pressed'),
  })

  expect(snapshot()).toEqual({
    fullscreenState: {
      className: 'web-ui-pane is-fullscreen',
      exitLabel: 'true',
    },
    restoredClassName: 'web-ui-pane',
    restoredLabel: 'false',
  })
})

it('opens the local web UI directly in the iframe and external link', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="crit"
        url="http://127.0.0.1:48761/review/sample"
        exposedUrl="https://public.example.test/review/sample"
        hostname="localhost"
        onClose={() => undefined}
      />,
    )
  })

  expect(relayTargets()).toEqual({
    iframe: 'http://127.0.0.1:48761/review/sample',
    link: 'http://127.0.0.1:48761/review/sample',
  })
})

it('opens the exposed web UI when watch-web is opened remotely', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Editor"
        url="http://127.0.0.1:48761/design/sample/"
        exposedUrl="https://public.example.test/design/sample/"
        hostname="watch.example.test"
        onClose={() => undefined}
      />,
    )
  })

  expect(relayTargets()).toEqual({
    iframe: 'https://public.example.test/design/sample/',
    link: 'https://public.example.test/design/sample/',
  })
})

it('shows the local URL when no remote URL is available', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Editor"
        url="http://127.0.0.1:48761/design/sample/"
        hostname="watch.example.test"
        onClose={() => undefined}
      />,
    )
  })

  expect(unavailablePaneState()).toEqual({
    iframe: null,
    link: null,
    status:
      'This interface is unavailable outside this machine. Local URL: http://127.0.0.1:48761/design/sample/',
    localUrl: 'http://127.0.0.1:48761/design/sample/',
  })
})
