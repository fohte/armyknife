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
        engine: 'claude',
        workType: 'PM',
        category: 'in_progress',
        isActive: true,
        kind: null,
        title: 'Root session',
        preview: 'Coordinating the task',
        elapsedSeconds: 300,
        task: null,
        repo: 'sample/repo',
        vibeMode: { mode: 'enabled' },
        decisionCount: 0,
        pullRequest: null,
      },
      {
        id: 'selected-session',
        parentId: 'root-session',
        engine: 'codex',
        workType: 'development',
        category: 'in_progress',
        isActive: true,
        kind: null,
        title: 'Selected session',
        preview: 'Implementing the interface',
        elapsedSeconds: 45,
        task: null,
        repo: 'sample/repo',
        vibeMode: { mode: 'inherited', ancestorSessionId: 'root-session' },
        decisionCount: 0,
        pullRequest: null,
      },
      {
        id: 'pending-session',
        parentId: 'selected-session',
        engine: 'claude',
        workType: 'research',
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
        vibeMode: { mode: 'disabled' },
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

export const PhoneStatusTreePresentation: Story = {
  name: 'The phone status tree shows the selected session and its descendants.',
  args: { mobilePresentation: true },
  decorators: [
    (Story) => (
      <div style={{ width: '390px', height: '844px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
}
