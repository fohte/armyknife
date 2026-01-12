# Changelog

## [0.1.10](https://github.com/fohte/armyknife/compare/v0.1.9...v0.1.10) (2026-01-12)


### Features

* **wm:** add git worktree management command ([#36](https://github.com/fohte/armyknife/issues/36)) ([259a8f2](https://github.com/fohte/armyknife/commit/259a8f29cf19cf9be13ddebe2c35bbe25943bc0e))

## [0.1.9](https://github.com/fohte/armyknife/compare/v0.1.8...v0.1.9) (2026-01-12)


### Features

* **name-branch:** add command to auto-generate branch names from task descriptions ([#41](https://github.com/fohte/armyknife/issues/41)) ([4a2e36c](https://github.com/fohte/armyknife/commit/4a2e36cd8fc3318c8d16356959b5c36d0cb5dbb7))

## [0.1.8](https://github.com/fohte/armyknife/compare/v0.1.7...v0.1.8) (2026-01-12)


### Bug Fixes

* **ai/pr-draft:** restore to focused pane at review command execution instead of source pane ([#42](https://github.com/fohte/armyknife/issues/42)) ([51c3a76](https://github.com/fohte/armyknife/commit/51c3a76c63bd5bb7d89a3012e93a5d3e609caf59))

## [0.1.7](https://github.com/fohte/armyknife/compare/v0.1.6...v0.1.7) (2026-01-10)


### Bug Fixes

* **ai/pr-draft:** use stable tmux IDs for window/pane restoration ([#35](https://github.com/fohte/armyknife/issues/35)) ([5c8eaae](https://github.com/fohte/armyknife/commit/5c8eaaeb53d4f602ffcae438b99f3d94944e4cd5))
* **pr-draft:** prevent accidental overwrite of existing draft files ([#33](https://github.com/fohte/armyknife/issues/33)) ([8be82a1](https://github.com/fohte/armyknife/commit/8be82a11f0f565a4135bcd8f6c1869c27332e98e))

## [0.1.6](https://github.com/fohte/armyknife/compare/v0.1.5...v0.1.6) (2026-01-08)


### Bug Fixes

* **update:** support gzip decompression ([#31](https://github.com/fohte/armyknife/issues/31)) ([708b7a6](https://github.com/fohte/armyknife/commit/708b7a6dae1a1b7b608e4c8e53fe8982c44559a8))

## [0.1.5](https://github.com/fohte/armyknife/compare/v0.1.4...v0.1.5) (2026-01-08)


### Bug Fixes

* **update:** fix tar archive extraction error in `a update` ([#29](https://github.com/fohte/armyknife/issues/29)) ([193f468](https://github.com/fohte/armyknife/commit/193f4683b130cc26184220c023c14ad5169998cf))

## [0.1.4](https://github.com/fohte/armyknife/compare/v0.1.3...v0.1.4) (2026-01-07)


### Bug Fixes

* **ai:** fix window title and private repo detection in pr-draft ([#26](https://github.com/fohte/armyknife/issues/26)) ([fdd4760](https://github.com/fohte/armyknife/commit/fdd47601eebd2964b263a468c6cfab368ad11724))

## [0.1.3](https://github.com/fohte/armyknife/compare/v0.1.2...v0.1.3) (2026-01-04)


### Features

* **ai:** add `a ai pr-draft` command ([#20](https://github.com/fohte/armyknife/issues/20)) ([6badeea](https://github.com/fohte/armyknife/commit/6badeeabe5f16fe3d050f619d5ac371dca12dd8c))

## [0.1.2](https://github.com/fohte/armyknife/compare/v0.1.1...v0.1.2) (2026-01-03)


### Features

* add automatic self-update ([#14](https://github.com/fohte/armyknife/issues/14)) ([43b650f](https://github.com/fohte/armyknife/commit/43b650fc9b19fe2198c4ab81c8159a67c98689c6))


### Bug Fixes

* **deps:** pin rust crate clap to v4.5.53 ([#6](https://github.com/fohte/armyknife/issues/6)) ([957a246](https://github.com/fohte/armyknife/commit/957a2465fa5f7dd926139bfc7d85570d458233e1))

## [0.1.1](https://github.com/fohte/armyknife/compare/v0.1.0...v0.1.1) (2026-01-02)


### Features

* add `--version` flag ([#3](https://github.com/fohte/armyknife/issues/3)) ([ee08fe9](https://github.com/fohte/armyknife/commit/ee08fe9e6dbef96b1ae05d164044e22cbcfc1806))
* initialize Rust CLI project ([#1](https://github.com/fohte/armyknife/issues/1)) ([eb6978f](https://github.com/fohte/armyknife/commit/eb6978f6695f9fdca269e473f8d96b489800c0a6))
