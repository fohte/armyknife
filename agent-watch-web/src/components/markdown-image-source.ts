// react-markdown normalizes image URLs before they reach custom components.
function normalizeMarkdownUri(value: string): string {
  let normalized = ''
  for (let index = 0; index < value.length;) {
    const code = value.charCodeAt(index)
    const character = value[index] ?? ''
    const next = value[index + 1] ?? ''
    const afterNext = value[index + 2] ?? ''
    if (
      character === '%' &&
      /^[\da-z]$/i.test(next) &&
      /^[\da-z]$/i.test(afterNext)
    ) {
      normalized += value.slice(index, index + 3)
      index += 3
      continue
    }
    if (code < 128 && /[!#$&-;=?-Z_a-z~]/.test(character)) {
      normalized += character
      index += 1
      continue
    }

    let encodedCharacter = character
    let characterLength = 1
    if (code >= 0xd800 && code <= 0xdbff) {
      const nextCode = value.charCodeAt(index + 1)
      if (nextCode >= 0xdc00 && nextCode <= 0xdfff) {
        encodedCharacter = value.slice(index, index + 2)
        characterLength = 2
      } else {
        encodedCharacter = '\uFFFD'
      }
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      encodedCharacter = '\uFFFD'
    }
    normalized += encodeURIComponent(encodedCharacter)
    index += characterLength
  }
  return normalized
}

function normalizeMarkdownImagePath(value: string): string {
  let unescaped = ''
  for (let index = 0; index < value.length;) {
    const character = value[index] ?? ''
    const next = value[index + 1] ?? ''
    const nextCode = next.charCodeAt(0)
    if (
      character === '\\' &&
      ((nextCode >= 0x21 && nextCode <= 0x2f) ||
        (nextCode >= 0x3a && nextCode <= 0x40) ||
        (nextCode >= 0x5b && nextCode <= 0x60) ||
        (nextCode >= 0x7b && nextCode <= 0x7e))
    ) {
      unescaped += next
      index += 2
      continue
    }
    unescaped += character
    index += 1
  }
  return normalizeMarkdownUri(unescaped)
}

export function getMappedImageSource(
  src: string,
  imageSources: Readonly<Record<string, string | null>> | undefined,
): string | null | undefined {
  if (imageSources === undefined) return undefined
  const matches = Object.entries(imageSources).filter(
    ([path]) => path === src || normalizeMarkdownImagePath(path) === src,
  )
  if (matches.length === 0) return undefined
  if (matches.length > 1) return null
  return matches[0]?.[1]
}

export function isUnmappedLocalImagePath(src: string): boolean {
  return (
    src.startsWith('/') &&
    !src.startsWith('//') &&
    !src.startsWith('/api/images/') &&
    !src.startsWith('/api/sessions/')
  )
}
