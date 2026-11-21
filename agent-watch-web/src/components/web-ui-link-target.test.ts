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
    description: 'a non-HTTP URL',
    href: 'mailto:canvas@example.invalid',
    expected: null,
  },
])('finds only registered web UI links: $description', ({ href, expected }) => {
  expect(findWebUiLink(href, [webUi])?.webUi ?? null).toEqual(expected)
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
