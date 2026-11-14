# Designs

Design files for agent-watch-web, edited with the [baoyu-design](https://github.com/JimLiu/baoyu-design) skills. [AGENTS.md](AGENTS.md) describes how designs are explored, decided, and recorded.

| Folder           | Skill                  | Contents                                                                           |
| ---------------- | ---------------------- | ---------------------------------------------------------------------------------- |
| `design-system/` | `claude-design-system` | Tokens, type, and static renditions of five components, written from the app's CSS |
| `ui/`            | `claude-design`        | The canvas: app screens, their parts, and design options under exploration        |
| `decisions/`     | -                      | One record per decided design theme, created with the first decision              |

## Install the skills

The skill files are not committed, because their upstream NOTICE grants no license for the parts adapted from Anthropic's material. Install them into this repository from its root:

```sh
DO_NOT_TRACK=1 npx skills@1.7.6 add JimLiu/baoyu-design --skill claude-design-system claude-design --agent claude-code -y
```

`skills-lock.json` records the installed versions, and this command rewrites it.

## Open a design

Each command starts a local editor and prints its URL:

```sh
node .claude/skills/claude-design-system/scripts/artifact.mjs open agent-watch-web/designs/design-system
node .claude/skills/claude-design/scripts/artifact.mjs open agent-watch-web/designs/ui
```
