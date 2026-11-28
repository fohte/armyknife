import type { Meta, StoryObj } from '@storybook/react-vite'

import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import { TodoCard } from '#components/todo-card'
import type { SessionLinkTarget } from '#types'

const linkedSession: SessionLinkTarget = {
  id: '11111111-1111-4111-8111-111111111111',
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}

const meta = {
  component: TodoCard,
  args: {
    title: 'Check the updated layout on a phone',
    detail:
      'Open the preview on a small screen and confirm the action stays visible.',
    resolution: null,
    onResolve: () => Promise.resolve(true),
    onFold: () => Promise.resolve(true),
  },
} satisfies Meta<typeof TodoCard>

export default meta
type Story = StoryObj<typeof meta>

export const TodoWaitsForTheUser: Story = {
  name: 'A to-do waits for the user to finish it',
}

export const TodoIsFoldedForLater: Story = {
  name: 'A folded to-do shows only its title',
  args: { folded: true },
}

export const TodoWasAddedFolded: Story = {
  name: 'A to-do added folded is marked for a later check',
  args: { folded: true, addedFolded: true },
}

export const TodoWasAddedFoldedOnPhone: Story = {
  name: 'A phone shows an added-folded to-do in two rows',
  args: { folded: true, addedFolded: true },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

export const TodoShowsMarkdownDetail: Story = {
  name: 'A to-do detail formats links and emphasis as markdown',
  decorators: [
    (Story) => (
      <MarkdownSessionLinksProvider
        sessions={[linkedSession]}
        onSelectSession={() => undefined}
      >
        <Story />
      </MarkdownSessionLinksProvider>
    ),
  ],
  args: {
    detail: `Review ${linkedSession.id} and [the example preview](https://example.test) before launch.\n\nKeep **the primary action visible** on small screens.`,
  },
}

export const TodoShowsMarkdownCodeBlock: Story = {
  name: 'A to-do code block includes the copy toolbar',
  args: {
    detail: '~~~bash\nprintf "ready\\n"\n~~~',
  },
}

export const TodoHasNoAdditionalDetail: Story = {
  name: 'A to-do shows only its title when it has no detail',
  args: { detail: null },
}

export const TodoIsCompleted: Story = {
  name: 'A completed to-do shows its title and saved comment compactly',
  args: {
    resolution: 'completed',
    defaultComment: 'The [sample](https://example.test) is **ready**.',
  },
}

export const TodoShowsASavedImage: Story = {
  name: 'A completed to-do shows its attached image in the saved comment',
  args: {
    resolution: 'completed',
    defaultComment: '![Sample screenshot](/card-comment-attachment.svg)',
  },
}

export const TodoIsCompletedWithoutAComment: Story = {
  name: 'A completed to-do without a comment shows only its title',
  args: { resolution: 'completed', defaultComment: '' },
}

export const TodoIsRejected: Story = {
  name: 'A rejected to-do shows its reason and title distinctly',
  args: { resolution: 'rejected', defaultComment: 'This is no longer needed.' },
}

export const TodoIsWithdrawn: Story = {
  name: 'A withdrawn to-do shows only its heading and title, even with a saved comment',
  args: { resolution: 'withdrawn', defaultComment: 'The plan changed.' },
}
