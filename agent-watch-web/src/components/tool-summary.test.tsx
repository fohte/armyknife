import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { ToolSummary } from '#components/tool-summary'

it('renders a collapsed summary without changing its text content', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary summary="2 actions · 1 file edited" operations={[]} />,
    ),
  ).toBe(
    '<details class="chat-tool-summary"><summary>2 actions · 1 file edited</summary></details>',
  )
})

it('renders a failed count separately from the summary text', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary summary="3 actions · 2 failed" operations={[]} />,
    ),
  ).toBe(
    '<details class="chat-tool-summary"><summary><span class="chat-tool-summary-text">3 actions</span><span class="chat-tool-summary-failed">· 2 failed</span></summary></details>',
  )
})

it('renders expanded operations as tool, target, and status rows', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="2 actions · 1 file edited"
        operations={[
          { tool: 'Read', target: 'src/feature.rs' },
          { tool: 'exec', target: 'printf hello' },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" open=""><summary>2 actions · 1 file edited</summary><ul class="chat-tool-operations"><li><span class="chat-tool-name">Read</span><code class="chat-tool-target">src/feature.rs</code></li><li><span class="chat-tool-name">exec</span><code class="chat-tool-target">printf hello</code></li></ul></details>',
  )
})

it('keeps long targets as plain text for CSS ellipsis', () => {
  const target = `echo ${'x'.repeat(241)}`

  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="1 action"
        operations={[{ tool: 'Bash', target }]}
        open
      />,
    ),
  ).toBe(
    `<details class="chat-tool-summary" open=""><summary>1 action</summary><ul class="chat-tool-operations"><li><span class="chat-tool-name">Bash</span><code class="chat-tool-target">${target}</code></li></ul></details>`,
  )
})
