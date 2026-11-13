import type { Meta, StoryObj } from '@storybook/react-vite'

import { SyntaxCodeBlock } from '#components/syntax-code-block'

const meta = {
  component: SyntaxCodeBlock,
  args: {
    language: 'bash',
    code: `set -euo pipefail
printf '%s\\n' 'Preparing preview'
git add ./sample-config.yml && git commit -m "$(cat <<'EOF'
chore: refresh sample config

intent(sample): keep local settings in sync
EOF
)"`,
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
} satisfies Meta<typeof SyntaxCodeBlock>

export default meta
type Story = StoryObj<typeof meta>

export const BashHighlightsACommand: Story = {
  name: 'A Bash block keeps heredocs and line breaks visible',
}
