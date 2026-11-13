# Designs

Design files for agent-watch-web, edited with the [baoyu-design](https://github.com/JimLiu/baoyu-design) skills.

| Folder                       | Skill                  | Contents                                                                           |
| ---------------------------- | ---------------------- | ---------------------------------------------------------------------------------- |
| `agent-watch-design-system/` | `claude-design-system` | Tokens, type, and static renditions of five components, written from the app's CSS |
| `mobile-session-top-bar/`    | `claude-design`        | The agent watch UI mock screens, plus three mobile top bar redesign options        |

The design system is written by hand from `agent-watch-web/src`. It does not follow code changes by itself.

## Install the skills

The skill files are not committed, because their upstream NOTICE grants no license for the parts adapted from Anthropic's material. Install them into this repository from its root:

```sh
DO_NOT_TRACK=1 npx skills@1.7.6 add JimLiu/baoyu-design --skill claude-design-system claude-design --agent claude-code -y
```

`skills-lock.json` records the installed versions, and this command rewrites it.

## Open a design

Each command starts a local editor and prints its URL:

```sh
node .claude/skills/claude-design-system/scripts/artifact.mjs open agent-watch-web/designs/agent-watch-design-system
node .claude/skills/claude-design/scripts/artifact.mjs open agent-watch-web/designs/mobile-session-top-bar
```

The canvas carries its own copy of the design system's tokens. After changing the design system, update the copy:

```sh
node .claude/skills/claude-design/scripts/artifact.mjs ds-sync agent-watch-web/designs/mobile-session-top-bar agentwatch
```
