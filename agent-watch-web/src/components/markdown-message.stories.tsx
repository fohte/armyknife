import type { Meta, StoryObj } from '@storybook/react-vite'

import { MarkdownMessage } from '#components/markdown-message'

const meta = {
  component: MarkdownMessage,
  args: {
    children: `## Retry policy

Use \`retry()\` with the [request guide](https://example.com/guide).

- Retry transient failures
- Stop after three attempts

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
  name: 'A message formats headings, lists, tables, links, and code',
}

export const RawHtmlStaysText: Story = {
  name: 'Raw HTML is shown as text instead of rendered',
  args: {
    children: 'This tag stays visible: <strong>not bold</strong>.',
  },
}
