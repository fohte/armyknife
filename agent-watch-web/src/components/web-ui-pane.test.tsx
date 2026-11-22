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

it('shows the URL as text beside the desktop web UI controls', () => {
  const targetUrl = 'https://review.example.test/review/sample'
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Review"
        url={targetUrl}
        targetUrlOverride={targetUrl}
        onClose={() => undefined}
      />,
    )
  })

  const headerState = () => ({
    name: container.querySelector('.web-ui-pane-title')?.textContent,
    url: container.querySelector('.web-ui-pane-url')?.textContent,
    externalLink: container
      .querySelector<HTMLAnchorElement>('a[aria-label="Open in a new tab"]')
      ?.getAttribute('href'),
    fullscreenButton: container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Show Review fullscreen"]',
      )
      ?.getAttribute('aria-label'),
    closeButton: container
      .querySelector<HTMLButtonElement>('button[aria-label="Close web UI"]')
      ?.getAttribute('aria-label'),
  })

  expect(headerState()).toEqual({
    name: 'Review',
    url: targetUrl,
    externalLink: targetUrl,
    fullscreenButton: 'Show Review fullscreen',
    closeButton: 'Close web UI',
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

it('shows the phone web UI URL, external link, and close action', () => {
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
        size="phone"
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
  function phoneHeaderState() {
    return {
      role: container.querySelector('aside')?.getAttribute('role'),
      name: container.querySelector('.web-ui-pane-title')?.textContent,
      url: container.querySelector('.web-ui-pane-url')?.textContent,
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
      fullscreenButton: container.querySelector(
        'button[aria-label*="fullscreen"]',
      ),
    }
  }
  expect(phoneHeaderState()).toEqual({
    role: 'dialog',
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
    fullscreenButton: null,
  })
})

it('shows the external link without fullscreen controls in a phone status tab', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Canvas editor"
        url="https://example.test/design/"
        targetUrlOverride="https://example.test/design/"
        previewMessage="The embedded design page is shown here."
        size="phone"
        onClose={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    role: container.querySelector('aside')?.getAttribute('role'),
    fullscreen: container
      .querySelector('.web-ui-pane')
      ?.classList.contains('is-fullscreen'),
    externalLink: container
      .querySelector<HTMLAnchorElement>('a[aria-label="Open in a new tab"]')
      ?.getAttribute('href'),
    fullscreenButton: container.querySelector(
      'button[aria-label*="fullscreen"]',
    ),
  })
  expect(snapshot()).toEqual({
    role: null,
    fullscreen: false,
    externalLink: 'https://example.test/design/',
    fullscreenButton: null,
  })
})

it('closes the phone web UI when its target URL is unavailable', () => {
  const onClose = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <WebUiPane
        name="Canvas editor"
        url="http://127.0.0.1:43123/studio/pages/sample"
        hostname="watch.example.test"
        onClose={onClose}
        defaultFullscreen
        size="phone"
      />,
    )
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('button[aria-label="Close web UI"]')
      ?.click()
  })

  function unavailableMobileHeaderState() {
    return {
      iframe: container.querySelector('iframe')?.getAttribute('src') ?? null,
      status: container.querySelector('[role="status"]')?.textContent,
      url: container.querySelector('.web-ui-pane-url')?.textContent,
      externalLink: container.querySelector('a'),
      closeButtonLabel: container
        .querySelector<HTMLButtonElement>('button[aria-label="Close web UI"]')
        ?.getAttribute('aria-label'),
      closeCalled: onClose.mock.calls.length,
    }
  }

  expect(unavailableMobileHeaderState()).toEqual({
    iframe: null,
    status:
      'This interface is unavailable outside this machine. Local URL: http://127.0.0.1:43123/studio/pages/sample',
    url: 'http://127.0.0.1:43123/studio/pages/sample',
    externalLink: null,
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
