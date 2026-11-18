import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import { EndedSessionBanner } from '#components/ended-session-banner'

const desktopDecorator: Decorator = (Story) => (
  <div className="conversation-pane">
    <Story />
  </div>
)

const phoneDecorator: Decorator = (Story) => (
  <div className="conversation-pane" data-mobile="true">
    <Story />
  </div>
)

const meta = {
  component: EndedSessionBanner,
  args: {
    endedAt: '2026-01-01T05:32:00.000Z',
    isMobile: false,
    isResuming: false,
    canResume: true,
    onResume: () => undefined,
  },
} satisfies Meta<typeof EndedSessionBanner>

export default meta
type Story = StoryObj<typeof meta>

export const EndedSessionCanBeResumed: Story = {
  name: 'An ended session offers a resume action',
  decorators: [desktopDecorator],
}

export const EndedSessionIsResuming: Story = {
  name: 'An ended session shows that it is resuming',
  args: { isResuming: true },
  decorators: [desktopDecorator],
}

export const EndedPhoneSessionCanBeResumed: Story = {
  name: 'The phone layout places the resume action below the status',
  args: { isMobile: true },
  decorators: [phoneDecorator],
}

export const EndedPhoneSessionIsResuming: Story = {
  name: 'The phone layout disables the action while the session is resuming',
  args: { isMobile: true, isResuming: true },
  decorators: [phoneDecorator],
}
