import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { ToolSummary } from '#components/tool-summary'

it('renders a collapsed summary without changing its text content', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary summary="2 actions · 1 file edited" operations={[]} />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="false"><summary>2 actions · 1 file edited</summary></details>',
  )
})

it('renders a failed count separately from the summary text', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary summary="3 actions · 2 failed" operations={[]} />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="false"><summary><span class="chat-tool-summary-text">3 actions</span><span class="chat-tool-summary-failed">· 2 failed</span></summary></details>',
  )
})

it('renders grouped tool activity as the collapsed row', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="Edit artifact-one.rs, artifact-two.rs  Read ×3  Grep ×1  Bash ×1 · 1 failed"
        operations={[
          { tool: 'Edit', target: 'artifact-one.rs' },
          { tool: 'Edit', target: 'artifact-two.rs' },
          { tool: 'Read', target: 'one.rs' },
          { tool: 'Read', target: 'two.rs' },
          { tool: 'Read', target: 'three.rs' },
          { tool: 'Grep', target: 'TODO' },
          { tool: 'Bash', target: 'cargo test', failed: true },
        ]}
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true"><summary><span class="chat-tool-summary-text">Edit artifact-one.rs, artifact-two.rs  Read ×3  Grep ×1  Bash ×1</span><span class="chat-tool-summary-failed">· 1 failed</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">artifact-one.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">artifact-two.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">one.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">two.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">three.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Grep</span><code class="chat-tool-target">TODO</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="true"><span class="chat-tool-name">Bash</span><code class="chat-tool-target">cargo test</code><span class="chat-tool-operation-status" role="img" aria-label="Failed">✕</span></li></ul></details>',
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
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span>2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">src/feature.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">exec</span><code class="chat-tool-target">printf hello</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
  )
})

it('omits empty targets from expanded operation rows', () => {
  const operations = [
    { tool: 'Read', target: null },
    { tool: 'Edit', target: '' },
  ]

  expect(
    renderToStaticMarkup(
      <ToolSummary summary="2 actions" operations={operations} open />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span>2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="false" data-failed="false"><span class="chat-tool-name">Read</span><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="false" data-failed="false"><span class="chat-tool-name">Edit</span><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
  )
})

it('keeps failure status attached to the matching operation', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="2 tool calls · 1 failed"
        operations={[
          { tool: 'Read', target: 'src/feature.rs', failed: true },
          { tool: 'Edit', target: 'src/feature.rs', failed: false },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span>2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="true"><span class="chat-tool-name">Read</span><code class="chat-tool-target">src/feature.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Failed">✕</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">src/feature.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
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
    `<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span>1 tool call</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Bash</span><code class="chat-tool-target">${target}</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>`,
  )
})
