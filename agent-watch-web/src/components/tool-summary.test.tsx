import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { ToolSummary } from '#components/tool-summary'

it('renders each tool with its target', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="2 actions · 1 file edited"
        operations={[
          { tool: 'Read', target: 'src/feature.rs' },
          { tool: 'exec', target: 'cargo test -p sample' },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" open=""><summary>2 actions · 1 file edited</summary><ul class="chat-tool-operations"><li><span class="chat-tool-name">Read</span><code>src/feature.rs</code></li><li><span class="chat-tool-name">exec</span><code>cargo test -p sample</code></li></ul></details>',
  )
})
