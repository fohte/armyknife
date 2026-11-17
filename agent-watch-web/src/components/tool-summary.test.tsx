import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { summarizeToolGroupItems, ToolSummary } from '#components/tool-summary'

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
          { tool: 'Edit', target: 'artifact-one.rs', failed: false },
          { tool: 'Edit', target: 'artifact-two.rs', failed: false },
          { tool: 'Read', target: 'one.rs', failed: false },
          { tool: 'Read', target: 'two.rs', failed: false },
          { tool: 'Read', target: 'three.rs', failed: false },
          { tool: 'Grep', target: 'TODO', failed: false },
          { tool: 'Bash', target: 'fictional failure command', failed: true },
        ]}
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true"><summary><span class="chat-tool-summary-text">Edit artifact-one.rs, artifact-two.rs  Read ×3  Grep ×1  Bash ×1</span><span class="chat-tool-summary-failed">· 1 failed</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">artifact-one.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">artifact-two.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">one.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">two.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">three.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Grep</span><code class="chat-tool-target">TODO</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="true"><span class="chat-tool-name">Bash</span><code class="chat-tool-target">fictional failure command</code><span class="chat-tool-operation-status" role="img" aria-label="Failed">✕</span></li></ul></details>',
  )
})

it('renders expanded operations as tool, target, and status rows', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="2 actions · 1 file edited"
        operations={[
          { tool: 'Read', target: 'src/feature.rs', failed: false },
          { tool: 'exec', target: 'printf hello', failed: false },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">src/feature.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">exec</span><code class="chat-tool-target">printf hello</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
  )
})

it('renders thoughts and tools in the supplied group order', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="Thought ×1 · Read ×1"
        operations={[
          { tool: 'Read', target: 'fixture/source.rs', failed: false },
        ]}
        items={[
          { type: 'thinking', text: 'Review the fictional request.' },
          {
            type: 'tool_use',
            tool: 'Read',
            target: 'fixture/source.rs',
            failed: false,
          },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">1 thought · 1 tool call</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the fictional request.</span><span class="chat-tool-thought-length">29 chars</span></summary><p class="chat-tool-thought-full">Review the fictional request.</p></details></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">fixture/source.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
  )
})

it('summarizes one edit by file name and counts the other tools', () => {
  expect(
    summarizeToolGroupItems([
      { type: 'thinking', text: 'Review the first fictional requirement.' },
      { type: 'thinking', text: 'Check the second fictional requirement.' },
      {
        type: 'tool_use',
        tool: 'Edit',
        target: 'fixture/client.ts',
        failed: false,
      },
      {
        type: 'tool_use',
        tool: 'Read',
        target: 'fixture/client.ts',
        failed: false,
      },
      {
        type: 'tool_use',
        tool: 'Read',
        target: 'fixture/settings.ts',
        failed: false,
      },
      {
        type: 'tool_use',
        tool: 'Shell',
        target: 'printf fixture',
        failed: false,
      },
    ]),
  ).toBe('Thought ×2 · Edit client.ts · Read ×2 · Shell ×1')
})

it.each([
  { tool: 'Write', target: 'fixture/client.ts' },
  { tool: 'MultiEdit', target: 'fixture/client.ts' },
  { tool: 'NotebookEdit', target: 'fixture/client.ts' },
  { tool: 'apply_patch', target: 'fixture/client.ts' },
  { tool: 'write_file', target: 'fixture/client.ts' },
  { tool: 'edit_file', target: 'fixture/client.ts' },
  { tool: 'FileChange', target: 'fixture/client.ts' },
  { tool: 'fileChange', target: 'fixture/client.ts' },
])('summarizes one $tool call by its file name', ({ tool, target }) => {
  expect(
    summarizeToolGroupItems([
      { type: 'tool_use', tool, target, failed: false },
    ]),
  ).toBe('Edit client.ts')
})

it('counts multiple edits instead of showing only one file name', () => {
  expect(
    summarizeToolGroupItems([
      {
        type: 'tool_use',
        tool: 'Edit',
        target: 'fixture/client.ts',
        failed: false,
      },
      {
        type: 'tool_use',
        tool: 'Edit',
        target: 'fixture/settings.ts',
        failed: false,
      },
    ]),
  ).toBe('Edit ×2')
})

it('renders thinking without tools as a collapsible group', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="Thought ×2"
        operations={[]}
        items={[
          { type: 'thinking', text: 'Review the first fictional detail.' },
          { type: 'thinking', text: 'Review the second fictional detail.' },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">2 thoughts</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the first fictional detail.</span><span class="chat-tool-thought-length">34 chars</span></summary><p class="chat-tool-thought-full">Review the first fictional detail.</p></details></li><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the second fictional detail.</span><span class="chat-tool-thought-length">35 chars</span></summary><p class="chat-tool-thought-full">Review the second fictional detail.</p></details></li></ul></details>',
  )
})

it('counts grouped thoughts and tool calls in the expanded heading', () => {
  const items = [
    { type: 'thinking' as const, text: 'Review the first fictional detail.' },
    { type: 'thinking' as const, text: 'Review the second fictional detail.' },
    {
      type: 'tool_use' as const,
      tool: 'Read',
      target: 'fixture/a.rs',
      failed: false,
    },
    {
      type: 'tool_use' as const,
      tool: 'Read',
      target: 'fixture/b.rs',
      failed: false,
    },
    {
      type: 'tool_use' as const,
      tool: 'Edit',
      target: 'fixture/c.rs',
      failed: false,
    },
    {
      type: 'tool_use' as const,
      tool: 'Shell',
      target: 'echo fixture',
      failed: false,
    },
  ]

  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="Thought ×2 · Read ×2 · Edit ×1 · Shell ×1"
        operations={[]}
        items={items}
        open
        showOperationStatus={false}
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">2 thoughts · 4 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the first fictional detail.</span><span class="chat-tool-thought-length">34 chars</span></summary><p class="chat-tool-thought-full">Review the first fictional detail.</p></details></li><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the second fictional detail.</span><span class="chat-tool-thought-length">35 chars</span></summary><p class="chat-tool-thought-full">Review the second fictional detail.</p></details></li><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">fixture/a.rs</code></li><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">fixture/b.rs</code></li><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">fixture/c.rs</code></li><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Shell</span><code class="chat-tool-target">echo fixture</code></li></ul></details>',
  )
})

it('omits a thought count when the expanded group only has tool calls', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="Read ×1"
        operations={[]}
        items={[
          {
            type: 'tool_use',
            tool: 'Read',
            target: 'fixture/source.rs',
            failed: false,
          },
        ]}
        open
        showOperationStatus={false}
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">1 tool call</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">fixture/source.rs</code></li></ul></details>',
  )
})

it('omits empty targets from expanded operation rows', () => {
  const operations = [
    { tool: 'Read', target: null, failed: false },
    { tool: 'Edit', target: '', failed: false },
  ]

  expect(
    renderToStaticMarkup(
      <ToolSummary summary="2 actions" operations={operations} open />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="false" data-failed="false"><span class="chat-tool-name">Read</span><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li><li data-has-target="false" data-failed="false"><span class="chat-tool-name">Edit</span><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
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
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="true"><span class="chat-tool-name">Read</span><code class="chat-tool-target">src/feature.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Failed">✕</span></li><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Edit</span><code class="chat-tool-target">src/feature.rs</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>',
  )
})

it('keeps long targets as plain text for CSS ellipsis', () => {
  const target = `echo ${'x'.repeat(241)}`

  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="1 action"
        operations={[{ tool: 'Bash', target, failed: false }]}
        open
      />,
    ),
  ).toBe(
    `<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">1 tool call</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false"><span class="chat-tool-name">Bash</span><code class="chat-tool-target">${target}</code><span class="chat-tool-operation-status" role="img" aria-label="Succeeded">✓</span></li></ul></details>`,
  )
})

it('omits unknown status when the summary only has an aggregate failure count', () => {
  expect(
    renderToStaticMarkup(
      <ToolSummary
        summary="2 actions · 1 failed"
        operations={[
          { tool: 'Read', target: 'artifact-one.rs' },
          { tool: 'Bash', target: 'fixture command', failed: true },
        ]}
        open
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" data-has-operations="true" open=""><summary><span class="chat-tool-summary-text">2 tool calls</span><span class="chat-tool-summary-collapse">‹ Collapse</span></summary><ul class="chat-tool-operations"><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">artifact-one.rs</code></li><li data-has-target="true" data-failed="true"><span class="chat-tool-name">Bash</span><code class="chat-tool-target">fixture command</code><span class="chat-tool-operation-status" role="img" aria-label="Failed">✕</span></li></ul></details>',
  )
})
