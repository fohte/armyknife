import { describe, expect, it } from 'vitest'

import { createQuitLifecycle } from './quit-lifecycle.js'

describe('createQuitLifecycle', () => {
  it('keeps retrying when the user keeps editing', () => {
    const actions: string[] = []
    const lifecycle = createQuitLifecycle(() => actions.push('stop-retrying'))

    lifecycle.beforeQuit()
    const shouldAllowUnload = lifecycle.rendererPreventedUnload(false)

    expect({
      isQuitting: lifecycle.isQuitting(),
      shouldAllowUnload,
      actions,
    }).toEqual({
      isQuitting: false,
      shouldAllowUnload: false,
      actions: [],
    })
  })

  it('stops retrying after the user discards the unsent message', () => {
    const actions: string[] = []
    const lifecycle = createQuitLifecycle(() => actions.push('stop-retrying'))

    lifecycle.beforeQuit()
    const shouldAllowUnload = lifecycle.rendererPreventedUnload(true)
    lifecycle.willQuit()

    expect({
      isQuitting: lifecycle.isQuitting(),
      shouldAllowUnload,
      actions,
    }).toEqual({
      isQuitting: true,
      shouldAllowUnload: true,
      actions: ['stop-retrying'],
    })
  })
})
