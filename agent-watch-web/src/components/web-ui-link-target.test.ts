import { expect, it } from 'vitest'

import type { WebUiLinkTarget } from '#components/markdown-session-links'
import {
  findWebUiLink,
  webUiLinkTargetUrl,
} from '#components/web-ui-link-target'

const webUi: WebUiLinkTarget = {
  name: 'Canvas editor',
  url: 'http://localhost:43123/studio/',
  exposedUrl: 'https://canvas.example.invalid/studio/',
}

it.each([
  {
    description: 'registered local URL with localhost alias',
    href: 'http://127.0.0.1:43123/studio/pages/sample#page-fakehash',
    expected: webUi,
  },
  {
    description: 'registered exposed URL',
    href: 'https://canvas.example.invalid/studio/pages/sample#page-fakehash',
    expected: webUi,
  },
  {
    description: 'a different port',
    href: 'http://localhost:43124/studio/pages/sample',
    expected: null,
  },
  {
    description: 'a neighboring path',
    href: 'http://localhost:43123/studio-other/page',
    expected: null,
  },
  {
    description: 'a root link to a pathful non-Crit web UI',
    href: 'http://localhost:43123/',
    expected: null,
  },
  {
    description: 'a non-HTTP URL',
    href: 'mailto:canvas@example.invalid',
    expected: null,
  },
])('finds only registered web UI links: $description', ({ href, expected }) => {
  expect(findWebUiLink(href, [webUi])?.webUi ?? null).toEqual(expected)
})

it('routes Crit preview root links through the registered preview URL', () => {
  const critPreview: WebUiLinkTarget = {
    name: 'crit',
    url: 'http://localhost:43123/preview',
    exposedUrl: 'https://crit.example.invalid/preview',
  }
  const href = 'http://localhost:43123/'

  expect(webUiLinkTargetUrl(critPreview, href, 'watch.example.invalid')).toBe(
    'https://crit.example.invalid/preview',
  )
})

it('prefers the most specific registered path when registrations overlap', () => {
  const broad: WebUiLinkTarget = {
    name: 'Studio root',
    url: 'http://localhost:43123/',
  }
  const specific: WebUiLinkTarget = {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/',
  }
  const href = 'http://localhost:43123/studio/pages/sample#page-fixture'
  function getMatch() {
    const match = findWebUiLink(href, [broad, specific])
    return {
      registeredUrl: match?.registeredUrl.href ?? null,
      webUi: match?.webUi ?? null,
    }
  }

  expect(getMatch()).toEqual({
    registeredUrl: 'http://localhost:43123/studio/',
    webUi: specific,
  })
})

it('maps a deep local link to the exposed web UI URL without losing its fragment', () => {
  expect(
    webUiLinkTargetUrl(
      webUi,
      'http://127.0.0.1:43123/studio/pages/sample?mode=edit#page-fakehash',
      'watch.example.invalid',
    ),
  ).toBe(
    'https://canvas.example.invalid/studio/pages/sample?mode=edit#page-fakehash',
  )
})

it.each([
  {
    description: 'keeps configured query parameters without link overrides',
    href: 'http://localhost:43123/studio/pages/sample',
    expected:
      'https://canvas.example.invalid/studio/pages/sample?token=fixture',
  },
  {
    description: 'merges link query parameters over configured values',
    href: 'http://localhost:43123/studio/pages/sample?mode=edit#page-fakehash',
    expected:
      'https://canvas.example.invalid/studio/pages/sample?token=fixture&mode=edit#page-fakehash',
  },
])('preserves configured query data: $description', ({ href, expected }) => {
  const webUiWithQuery: WebUiLinkTarget = {
    ...webUi,
    url: 'http://localhost:43123/studio/?token=fixture',
    exposedUrl: 'https://canvas.example.invalid/studio/?token=fixture',
  }
  expect(
    webUiLinkTargetUrl(webUiWithQuery, href, 'watch.example.invalid'),
  ).toBe(expected)
})
