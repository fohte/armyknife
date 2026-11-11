import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionTreePanel } from '#components/session-tree-panel'

const meta = {
  title: 'Agent Watch/Session Tree Panel',
  component: SessionTreePanel,
  args: {
    selectedSessionId: 'selected-session',
    onSelectSession: () => undefined,
    sessions: [
      {
        id: 'root-session',
        parentId: null,
        category: 'in_progress',
        isActive: true,
        kind: null,
        title: 'Root session',
        preview: 'Coordinating the task',
        elapsedSeconds: 300,
        task: null,
        repo: 'sample/repo',
        decisionCount: 0,
        pullRequest: null,
      },
      {
        id: 'selected-session',
        parentId: 'root-session',
        category: 'in_progress',
        isActive: true,
        kind: null,
        title: 'Selected session',
        preview: 'Implementing the interface',
        elapsedSeconds: 45,
        task: null,
        repo: 'sample/repo',
        decisionCount: 0,
        pullRequest: null,
      },
      {
        id: 'pending-session',
        parentId: 'selected-session',
        category: 'needs_action',
        isActive: true,
        kind: 'Question',
        title: 'Pending child',
        preview: 'Waiting for an answer',
        elapsedSeconds: 8,
        task: {
          id: 'task-id',
          number: 17,
          title: 'Sample task',
          priority: null,
        },
        repo: 'sample/other-repo',
        decisionCount: 0,
        pullRequest: null,
      },
    ],
  },
} satisfies Meta<typeof SessionTreePanel>

export default meta
type Story = StoryObj<typeof meta>

export const SelectedSessionWithPendingChild: Story = {
  name: 'The selected session shows its pending child.',
}
