import type { Meta, StoryObj } from '@storybook/react-vite'

import { UpdateNotice } from '#components/update-notice'

const meta = {
  component: UpdateNotice,
  args: {
    version: '9.8.7',
    isMobile: false,
    onReload: () => undefined,
    onDismiss: () => undefined,
  },
} satisfies Meta<typeof UpdateNotice>

export default meta
type Story = StoryObj<typeof meta>

export const DesktopUpdateIsAvailable: Story = {
  name: 'The desktop shows a new version with a reload action.',
}

export const PhoneUpdateIsAvailable: Story = {
  name: 'The phone shows a compact update band after the app header.',
  args: { isMobile: true },
  parameters: { viewport: { defaultViewport: 'mobile2' } },
}
