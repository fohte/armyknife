import type { Meta, StoryObj } from '@storybook/react-vite'

import {
  MarkdownMessage,
  MarkdownSessionLinksProvider,
} from '#components/markdown-message'
import type { WebUiLinkTarget } from '#components/markdown-session-links'
import type { SessionLinkTarget } from '#types'

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
const maxPreviewItems = 7;
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

const linkedSession: SessionLinkTarget = {
  id: '11111111-1111-4111-8111-111111111111',
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}

export const KnownSessionIdsAreLinked: Story = {
  name: 'A known session ID links while an unknown ID stays plain',
  render: (args) => (
    <MarkdownSessionLinksProvider
      sessions={[linkedSession]}
      onSelectSession={() => undefined}
    >
      <MarkdownMessage {...args} />
    </MarkdownSessionLinksProvider>
  ),
  args: {
    children: `The session ${linkedSession.id} is active, while 22222222-2222-4222-8222-222222222222 is unknown.

Use \`${linkedSession.id}\` to open it.

~~~text
${linkedSession.id}
~~~`,
  },
}

const designWebUi: WebUiLinkTarget = {
  name: 'Canvas editor',
  url: 'http://localhost:43123/studio/',
  exposedUrl: 'https://canvas.example.invalid/studio/',
}

export const RegisteredWebUiLinksShowTheirTabName: Story = {
  name: 'A registered web UI link shows its tab name',
  render: (args) => (
    <MarkdownSessionLinksProvider
      sessions={[]}
      webUis={[designWebUi]}
      onOpenWebUi={() => undefined}
    >
      <MarkdownMessage {...args} />
    </MarkdownSessionLinksProvider>
  ),
  args: {
    children:
      'Open the [canvas page](http://localhost:43123/studio/pages/sample#page-fakehash) or the [external guide](https://example.invalid/guide).',
  },
}

export const InlineBangCommand: Story = {
  name: 'The message includes an inline command',
  args: {
    children: 'Run `! printf result` after review.',
    sessionId: 'sample-session',
  },
}

export const FencedBangCommands: Story = {
  name: 'The message includes several fenced commands',
  args: {
    children: '```sh\n! printf first\n! printf second\n```',
    sessionId: 'sample-session',
  },
}

export const IndentedBangCommand: Story = {
  name: 'The message includes an indented command',
  args: {
    children: '    ! printf result',
    sessionId: 'sample-session',
  },
}

export const CodeBlockWithoutLanguage: Story = {
  name: 'A code block without a language is labelled as text',
  args: {
    children: '```\nPlain text can be copied from this block.\n```',
  },
}

export const AssistantMessageCodeBlock: Story = {
  name: 'An assistant message shows the copy toolbar above its code',
  args: {
    className: 'chat-assistant-message',
    children: '```typescript\nconst value = 1\n```',
  },
}

export const UserMessageCodeBlock: Story = {
  name: 'A user message keeps the copy toolbar inside its bubble',
  args: {
    className: 'chat-user-message',
    children: "```bash\nprintf 'ready\\n'\n```",
  },
}

export const GenericMarkdownCodeBlock: Story = {
  name: 'A generic Markdown code block has a copy toolbar',
  args: {
    children: '~~~text\nRendered with a toolbar.\n~~~',
  },
}
