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

it.each([
  {
    name: 'matches localhost to a 127.0.0.1 watch-web host',
    url: 'http://localhost:48761/review/sample',
    hostname: '127.0.0.1',
    expectedUrl: 'http://127.0.0.1:48761/review/sample',
  },
  {
    name: 'matches 127.0.0.1 to a localhost watch-web host',
    url: 'http://127.0.0.1:48761/review/sample',
    hostname: 'localhost',
    expectedUrl: 'http://localhost:48761/review/sample',
  },
  {
    name: 'keeps a remote URL when watch-web is local',
    url: 'https://review.example.test:4321/review',
    hostname: 'localhost',
    expectedUrl: 'https://review.example.test:4321/review',
  },
  {
    name: 'keeps a malformed legacy URL without throwing',
    url: 'xhttp://127.0.0.1:48761/review',
    hostname: 'localhost',
    expectedUrl: 'xhttp://127.0.0.1:48761/review',
  },
])(
  'opens the local web UI with a matching host: $name',
  ({ url, hostname, expectedUrl }) => {
    root = createRoot(container)
    act(() => {
      root?.render(
        <WebUiPane
          name="crit"
          url={url}
          exposedUrl="https://public.example.test/review/sample"
          hostname={hostname}
          onClose={() => undefined}
        />,
      )
    })

    expect(relayTargets()).toEqual({
      iframe: expectedUrl,
      link: expectedUrl,
    })
  },
)

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

it('shows the mobile web UI URL and closes it from the header', () => {
  const targetUrl = 'http://localhost:43123/studio/pages/sample#page-fakehash'
  const onClose = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Canvas editor"
        url={targetUrl}
        targetUrlOverride={targetUrl}
        onClose={onClose}
        defaultFullscreen
        mobilePresentation
      />,
    )
  })

  const externalLink = container.querySelector<HTMLAnchorElement>(
    'a[aria-label="Open in a new tab"]',
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('button[aria-label="Close web UI"]')
      ?.click()
  })
  function mobileHeaderState() {
    return {
      hasBackButton:
        container.querySelector(
          'button[aria-label="Back to the conversation"]',
        ) !== null,
      name: container.querySelector('.web-ui-pane-title')?.textContent,
      url: container.querySelector('.web-ui-pane-mobile-url')?.textContent,
      externalLink: {
        href: externalLink?.getAttribute('href'),
        label: externalLink?.getAttribute('aria-label'),
        rel: externalLink?.getAttribute('rel'),
        target: externalLink?.getAttribute('target'),
      },
      closeButtonLabel: container
        .querySelector<HTMLButtonElement>('button[aria-label="Close web UI"]')
        ?.getAttribute('aria-label'),
      closeCalled: onClose.mock.calls.length,
    }
  }
  expect(mobileHeaderState()).toEqual({
    hasBackButton: false,
    name: 'Canvas editor',
    url: targetUrl,
    externalLink: {
      href: targetUrl,
      label: 'Open in a new tab',
      rel: 'noopener noreferrer',
      target: '_blank',
    },
    closeButtonLabel: 'Close web UI',
    closeCalled: 1,
  })
})

it('reopens the requested page after the iframe navigates', () => {
  const targetUrl = 'http://localhost:43123/studio/pages/sample#page-fixture'
  const renderWithRequest = (fullscreenRequest: number) => {
    act(() => {
      root?.render(
        <WebUiPane
          name="Canvas editor"
          url={targetUrl}
          targetUrlOverride={targetUrl}
          onClose={() => undefined}
          fullscreenRequest={fullscreenRequest}
        />,
      )
    })
  }

  root = createRoot(container)
  renderWithRequest(1)
  container
    .querySelector<HTMLIFrameElement>('iframe')
    ?.setAttribute('src', 'http://localhost:43123/studio/pages/other')
  renderWithRequest(2)

  function getIframeState() {
    return {
      src: container.querySelector('iframe')?.getAttribute('src') ?? null,
    }
  }
  expect(getIframeState()).toEqual({ src: targetUrl })
})
