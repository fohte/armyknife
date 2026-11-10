import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxFilterDialog } from '#components/inbox-filter-dialog'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'sample-question',
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    engine: 'Claude',
    title: 'Check the request retry policy',
    preview: null,
    elapsedSeconds: 10,
    unread: true,
    task: {
      id: 'sample-task',
      number: 123,
      title: 'Improve request reliability',
      priority: 1,
    },
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-review',
    category: 'later',
    isPaused: false,
    kind: 'Review',
    engine: 'Codex',
    title: 'Review service changes',
    preview: null,
    elapsedSeconds: 20,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
]

const meta = {
  component: InboxFilterDialog,
  args: {
    sessions,
    query: 'project:web-app',
    onQueryChange: () => undefined,
    onApply: () => undefined,
    onClose: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '100vw', height: '100vh', background: '#09090b' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof InboxFilterDialog>

export default meta
type Story = StoryObj<typeof meta>

export const ProjectAndKindValuesAreSelected: Story = {
  name: 'The dialog keeps the selected project beside its count',
}
