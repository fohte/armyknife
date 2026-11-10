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
          { tool: 'exec', target: 'printf hello' },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" open=""><summary>2 actions · 1 file edited</summary><ul class="chat-tool-operations"><li><span class="chat-tool-name">Read</span><code>src/feature.rs</code></li><li><span class="chat-tool-name">exec</span><code class="chat-tool-command hljs"><span class="hljs-built_in">printf</span> hello</code></li></ul></details>',
  )
})

it('renders the complete shell command after syntax highlighting', () => {
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
    `<details class="chat-tool-summary" open=""><summary>1 action</summary><ul class="chat-tool-operations"><li><span class="chat-tool-name">Bash</span><code class="chat-tool-command hljs"><span class="hljs-built_in">echo</span> ${'x'.repeat(241)}</code></li></ul></details>`,
  )
})

it('keeps shell commands plain until the summary is expanded', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="1 action"
        operations={[{ tool: 'Bash', target: 'printf hello' }]}
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary"><summary>1 action</summary><ul class="chat-tool-operations"><li><span class="chat-tool-name">Bash</span><code>printf hello</code></li></ul></details>',
  )
})
