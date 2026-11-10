import type { Meta, StoryObj } from '@storybook/react-vite'

import { MarkdownMessage } from '#components/markdown-message'
import type { Session } from '#types'

const meta = {
  component: MarkdownMessage,
  args: {
    children: `## Retry policy

Use \`retry()\` with the [request guide](https://example.com/guide).

- Retry transient failures
  - Wait before retrying
- Stop after three attempts

1. Check the request
   1. Confirm the response
   2. Record the result
2. Continue after approval

| Attempt | Delay |
| --- | --- |
| 1 | 100 ms |
| 2 | 200 ms |

~~~rust
let attempts = 3;
~~~`,
  },
  decorators: [
    (Story) => (
      <div
        style={{ maxWidth: '640px', padding: '24px', background: '#0a0a0a' }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MarkdownMessage>

export default meta
type Story = StoryObj<typeof meta>

export const FormatsCommonMarkdown: Story = {
  name: 'A message shows nested bullet and numbered lists',
}

export const RawHtmlStaysText: Story = {
  name: 'Raw HTML is shown as text instead of rendered',
  args: {
    children: 'This tag stays visible: <strong>not bold</strong>.',
  },
}

const linkedSession: Session = {
  id: '11111111-1111-4111-8111-111111111111',
  category: 'in_progress',
  isPaused: false,
  kind: 'Plan',
  title: 'Inspect the retry path',
  preview: null,
  elapsedSeconds: 30,
  unread: false,
  task: null,
  repo: 'example/toolkit',
  project: 'example toolkit',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

export const KnownSessionIdsAreLinked: Story = {
  name: 'A known session ID links while an unknown ID stays plain',
  args: {
    children: `The session ${linkedSession.id} is active, while 22222222-2222-4222-8222-222222222222 is unknown.

Use \`${linkedSession.id}\` to open it.

~~~text
${linkedSession.id}
~~~`,
    sessions: [linkedSession],
    onSelectSession: () => undefined,
  },
}
