let noExternalGitHubRefsPlugin = null
try {
  // Plugin file exists only in public repos (controlled by copier is_public parameter)
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  noExternalGitHubRefsPlugin =
    require('./commitlint-plugin-no-external-refs').default
} catch {
  // Plugin not available (private repo)
}

const config = {
  rules: {
    'body-leading-blank': [2, 'always'],
    'footer-leading-blank': [2, 'always'],
    'subject-full-stop': [2, 'never', '.'],
    ...(noExternalGitHubRefsPlugin && {
      'no-external-github-refs': [2, 'always'],
    }),
  },
  plugins: noExternalGitHubRefsPlugin ? [noExternalGitHubRefsPlugin] : [],
}

export default config
