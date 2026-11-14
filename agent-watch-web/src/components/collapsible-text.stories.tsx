import type { Meta, StoryObj } from '@storybook/react-vite'

import { CollapsibleText } from '#components/collapsible-text'

const meta = {
  component: CollapsibleText,
  args: {
    children:
      'Read each section and check that the layout remains clear on a narrow screen. Repeat the review after resizing the window to confirm the content still fits. Compare each card with the surrounding controls and keep the main action visible while reading. Check again after the viewport changes so long notes do not hide the choices that people need to make. Confirm that the complete text remains available when the reader chooses to expand the details.',
    expanded: false,
    onExpandedChange: () => undefined,
  },
} satisfies Meta<typeof CollapsibleText>

export default meta
type Story = StoryObj<typeof meta>

export const Collapsed: Story = {
  name: 'Long text shows two lines and offers to expand.',
}

export const Expanded: Story = {
  name: 'Long text is fully visible and offers to collapse.',
  args: { expanded: true },
}

export const ShortText: Story = {
  name: 'Short text fits without an expand button.',
  args: { children: 'Check that the layout is clear.' },
}
