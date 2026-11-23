import { expect, it } from 'vitest'

import { formatPermissionCommand } from '#components/permission-command'

it('wraps shell separators outside quoted strings and indents continuations', () => {
  const command =
    'printf "%s" "keep && and ||" && printf "left" || printf "fallback" | tr x y; printf done'

  expect(formatPermissionCommand(command)).toBe(
    'printf "%s" "keep && and ||" &&\n  printf "left" ||\n  printf "fallback" |\n  tr x y;\n  printf done',
  )
})

it('leaves command substitutions and backtick contents on their original line', () => {
  const command =
    "printf %s $(printf 'a && b' | sed 's/a/b/') && printf %s `printf 'x;y'`"

  expect(formatPermissionCommand(command)).toBe(
    "printf %s $(printf 'a && b' | sed 's/a/b/') &&\n  printf %s `printf 'x;y'`",
  )
})

it('keeps combined pipe operators intact', () => {
  expect(formatPermissionCommand('run-task |& log-output')).toBe(
    'run-task |&\n  log-output',
  )
})

it('does not split escaped shell separators', () => {
  const command = 'printf \\&\\& && printf done'

  expect(formatPermissionCommand(command)).toBe(
    'printf \\&\\& &&\n  printf done',
  )
})

it('keeps multiline commands unchanged', () => {
  const command = "cat <<'EOF'\nfirst && second | third\nEOF"

  expect(formatPermissionCommand(command)).toBe(command)
})
