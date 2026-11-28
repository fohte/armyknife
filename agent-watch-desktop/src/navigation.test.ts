import { describe, expect, it } from 'vitest'

import {
  classifyNavigation,
  isAllowedWatchUrl,
  isWatchAppPageUrl,
} from './navigation.js'

describe('classifyNavigation', () => {
  it.each([
    {
      name: 'allows same-origin navigation',
      currentUrl: 'http://127.0.0.1:4317/?session=first',
      targetUrl: 'http://127.0.0.1:4317/?session=second',
      opensWindow: false,
      expected: 'allow',
    },
    {
      name: 'opens same-origin PDF links in an app window',
      currentUrl: 'http://127.0.0.1:4317/',
      targetUrl: 'http://127.0.0.1:4317/files/example.pdf',
      opensWindow: true,
      expected: 'open-window',
    },
    {
      name: 'opens HTTP links externally',
      currentUrl: 'http://127.0.0.1:4317/',
      targetUrl: 'http://example.test/pull/123',
      opensWindow: false,
      expected: 'open-external',
    },
    {
      name: 'opens HTTPS links externally',
      currentUrl: 'http://127.0.0.1:4317/',
      targetUrl: 'https://example.test/pull/123',
      opensWindow: false,
      expected: 'open-external',
    },
    {
      name: 'opens mailto links externally',
      currentUrl: 'http://127.0.0.1:4317/',
      targetUrl: 'mailto:watch@example.test',
      opensWindow: true,
      expected: 'open-external',
    },
    {
      name: 'denies unsupported schemes',
      currentUrl: 'http://127.0.0.1:4317/',
      targetUrl: 'file:///etc/passwd',
      opensWindow: false,
      expected: 'deny',
    },
    {
      name: 'denies external popup requests from untrusted pages',
      currentUrl: 'https://example.test/',
      targetUrl: 'https://another.test/',
      opensWindow: true,
      expected: 'deny',
    },
    {
      name: 'denies malformed URLs',
      currentUrl: 'http://127.0.0.1:4317/',
      targetUrl: 'not a URL',
      opensWindow: false,
      expected: 'deny',
    },
  ])('$name', ({ currentUrl, targetUrl, opensWindow, expected }) => {
    expect(
      classifyNavigation(
        currentUrl,
        targetUrl,
        'http://127.0.0.1:4317',
        opensWindow,
      ),
    ).toEqual(expected)
  })
})

describe('isAllowedWatchUrl', () => {
  it.each([
    {
      name: 'matches the complete origin',
      url: 'http://127.0.0.1:4317/?session=first',
      expected: true,
    },
    {
      name: 'rejects a prefix-matching hostname',
      url: 'http://localhost.example.test:4317/',
      expected: false,
    },
    {
      name: 'rejects a different port',
      url: 'http://127.0.0.1:4318/',
      expected: false,
    },
  ])('$name', ({ url, expected }) => {
    expect(isAllowedWatchUrl(url, 'http://127.0.0.1:4317')).toEqual(expected)
  })
})

describe('isWatchAppPageUrl', () => {
  it.each([
    {
      name: 'matches the app page while ignoring its query and fragment',
      url: 'http://127.0.0.1:4317/?session=first#conversation',
      watchUrl: 'http://127.0.0.1:4317/',
      expected: true,
    },
    {
      name: 'rejects same-origin file content URLs',
      url: 'http://127.0.0.1:4317/api/sessions/example/files/file',
      watchUrl: 'http://127.0.0.1:4317/',
      expected: false,
    },
    {
      name: 'rejects pages at a different origin',
      url: 'https://example.test/',
      watchUrl: 'http://127.0.0.1:4317/',
      expected: false,
    },
  ])('$name', ({ url, watchUrl, expected }) => {
    expect(isWatchAppPageUrl(url, watchUrl)).toEqual(expected)
  })
})
