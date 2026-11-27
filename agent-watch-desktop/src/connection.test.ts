import { describe, expect, it } from 'vitest'

import { createConnectionController } from './connection.js'

describe('createConnectionController', () => {
  it('retries the failed session URL after the server responds', () => {
    const actions: string[] = []
    const controller = createConnectionController(
      'http://127.0.0.1:4317',
      'http://127.0.0.1:4317/',
      {
        showWaitingPage: () => actions.push('show-waiting'),
        loadWatchPage: (url) => actions.push(`load:${url}`),
      },
    )

    controller.mainFrameDidFail(
      'http://127.0.0.1:4317/?session=session-example',
    )
    controller.probe(false)
    controller.probe(true)

    expect(actions).toEqual([
      'show-waiting',
      'load:http://127.0.0.1:4317/?session=session-example',
    ])
  })

  it.each([
    {
      name: 'keeps a loaded page when its server probe fails',
      failedUrl: undefined,
      loadedUrl: 'http://127.0.0.1:4317/?session=session-example',
      serverResponded: false,
      expected: [],
    },
    {
      name: 'uses the configured URL when a failed URL is external',
      failedUrl: 'https://example.test/',
      loadedUrl: undefined,
      serverResponded: true,
      expected: ['show-waiting', 'load:http://127.0.0.1:4317/'],
    },
  ])('$name', ({ failedUrl, loadedUrl, serverResponded, expected }) => {
    const actions: string[] = []
    const controller = createConnectionController(
      'http://127.0.0.1:4317',
      'http://127.0.0.1:4317/',
      {
        showWaitingPage: () => actions.push('show-waiting'),
        loadWatchPage: (url) => actions.push(`load:${url}`),
      },
    )

    if (loadedUrl !== undefined) controller.mainFrameDidLoad(loadedUrl)
    if (failedUrl !== undefined) controller.mainFrameDidFail(failedUrl)
    controller.probe(serverResponded)

    expect(actions).toEqual(expected)
  })
})
