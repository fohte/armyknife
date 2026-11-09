import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { App } from '#App'

it('renders the agent watch prompt', () => {
  expect(renderToStaticMarkup(<App />)).toBe(
    '<h1 class="font-mono text-foreground">&gt; agent watch</h1>',
  )
})
