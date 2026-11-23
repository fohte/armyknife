export function formatPermissionCommand(command: string): string {
  if (command.includes('\n') || command.includes('\r')) return command

  let formatted = ''
  let quote: "'" | '"' | '`' | null = null
  let substitutionDepth = 0

  for (let index = 0; index < command.length; index += 1) {
    const character = command.charAt(index)

    if (quote !== null) {
      formatted += character
      if (quote !== "'" && character === '\\') {
        index += 1
        formatted += command.charAt(index)
      } else if (character === quote) {
        quote = null
      }
      continue
    }

    if (character === '\\') {
      formatted += character
      index += 1
      formatted += command.charAt(index)
      continue
    }

    if (character === "'" || character === '"' || character === '`') {
      quote = character
      formatted += character
      continue
    }

    if (character === '$' && command[index + 1] === '(') {
      substitutionDepth += 1
      formatted += '$('
      index += 1
      continue
    }

    if (substitutionDepth > 0) {
      if (character === '(') substitutionDepth += 1
      if (character === ')') substitutionDepth -= 1
      formatted += character
      continue
    }

    const operator = readSeparator(command, index)
    if (operator === null) {
      formatted += character
      continue
    }

    let next = index + operator.length
    while (command[next] === ' ' || command[next] === '\t') next += 1
    if (next === command.length) {
      formatted += operator
      index += operator.length - 1
      continue
    }

    formatted += `${operator}\n  `
    index = next - 1
  }

  return formatted
}

function readSeparator(command: string, index: number): string | null {
  const remaining = command.slice(index)
  if (remaining.startsWith('&&') || remaining.startsWith('||')) {
    return remaining.slice(0, 2)
  }
  if (remaining.startsWith('|&')) return '|&'
  if (remaining.startsWith('|')) return '|'
  if (remaining.startsWith(';;&')) return ';;&'
  if (remaining.startsWith(';;') || remaining.startsWith(';&')) {
    return remaining.slice(0, 2)
  }
  if (remaining.startsWith(';')) return ';'
  return null
}
