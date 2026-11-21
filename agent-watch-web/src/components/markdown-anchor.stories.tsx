import type { Meta, StoryObj } from '@storybook/react-vite'

import {
  MarkdownMessage,
  MarkdownSessionLinksProvider,
} from '#components/markdown-message'
import type { WebUiLinkTarget } from '#components/markdown-session-links'

const webUi: WebUiLinkTarget = {
  name: 'Canvas editor',
  url: 'https://canvas.example.invalid/studio/',
}

const meta = {
  component: MarkdownMessage,
  args: {
    children:
      '[![Fixture illustration](/fixture-illustration.png)](https://canvas.example.invalid/studio/pages/sample#page-fixture)',
  },
  decorators: [
    (Story) => (
      <MarkdownSessionLinksProvider
        sessions={[]}
        webUis={[webUi]}
        onOpenWebUi={() => undefined}
      >
        <div
          style={{ maxWidth: '640px', padding: '24px', background: '#0a0a0a' }}
        >
          <Story />
        </div>
      </MarkdownSessionLinksProvider>
    ),
  ],
} satisfies Meta<typeof MarkdownMessage>

export default meta
type Story = StoryObj<typeof meta>

export const LinkedImageTargetsARegisteredWebUi: Story = {
  name: 'A linked image has a separate action for its registered web UI',
}
