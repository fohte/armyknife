import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { SubagentReport } from '#components/subagent-report'
import type { ChatItem } from '#types'

const message = {
  type: 'subagent_report',
  agent_id: 'fixture-agent-id',
  agent_name: 'general-purpose',
  description: 'Review the fictional feature',
  text: '## Result\n\nThe requested change is complete.',
} satisfies Extract<ChatItem, { type: 'subagent_report' }>

const messageWithoutDescription = {
  type: 'subagent_report',
  agent_id: 'fixture-agent-id',
  agent_name: 'general-purpose',
  text: '## Result\n\nThe requested change is complete.',
} satisfies Extract<ChatItem, { type: 'subagent_report' }>

it('renders the task description with its report collapsed', () => {
  expect(
    renderToStaticMarkup(
      <SubagentReport
        message={message}
        onSelectSession={() => undefined}
        sessions={[]}
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary"><summary>Review the fictional feature</summary><div class="markdown-message"><h2>Result</h2>\n<p>The requested change is complete.</p></div></details>',
  )
})

it('uses the agent type name and renders the report body as markdown', () => {
  expect(
    renderToStaticMarkup(
      <SubagentReport
        message={messageWithoutDescription}
        onSelectSession={() => undefined}
        sessions={[]}
        defaultOpen
      />,
    ),
  ).toBe(
    '<details class="chat-tool-summary" open=""><summary>general-purpose</summary><div class="markdown-message"><h2>Result</h2>\n<p>The requested change is complete.</p></div></details>',
  )
})
