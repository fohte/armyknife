# Designs

Design files for agent-watch-web, edited with the [baoyu-design](https://github.com/JimLiu/baoyu-design) skills. [AGENTS.md](AGENTS.md) describes who edits what, and how a design goes from options to a decision.

| Folder             | Skill                  | Contents                                                                           |
| ------------------ | ---------------------- | ---------------------------------------------------------------------------------- |
| `design-system/`   | `claude-design-system` | Tokens, type, and static renditions of five components, written from the app's CSS |
| `screens/`         | `claude-design`        | Canvas of the screens and parts the app should have                                |
| `explore/<theme>/` | `claude-design`        | One canvas per design theme being decided                                          |
| `decisions/`       | -                      | Decided themes with their record and canvas; created with the first decision       |

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
node .claude/skills/claude-design/scripts/artifact.mjs open agent-watch-web/designs/screens
node .claude/skills/claude-design/scripts/artifact.mjs open agent-watch-web/designs/explore/topbar
```
