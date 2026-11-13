import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { SyntaxCodeBlock } from '#components/syntax-code-block'

it('renders code before the highlighter loads', () => {
  expect(
    renderToStaticMarkup(
      <SyntaxCodeBlock code={'echo "hello"'} language="bash" />,
    ),
  ).toBe(
    '<pre class="syntax-code-block" data-language="bash"><code class="language-bash">echo &quot;hello&quot;</code></pre>',
  )
})

it('leaves unsupported languages without syntax tokens', async () => {
  const { highlightCode } = await import('./syntax-highlighter')

  expect(await highlightCode('opaque preview', 'unknown-format')).toBeNull()
})
