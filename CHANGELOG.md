# Changelog

## [0.1.55](https://github.com/fohte/armyknife/compare/v0.1.54...v0.1.55) (2026-01-30)


### Bug Fixes

* **cc/watch:** optimize idle performance ([#181](https://github.com/fohte/armyknife/issues/181)) ([73f4a02](https://github.com/fohte/armyknife/commit/73f4a02f49d7b5e33ecb659931638cfbd51a0d00))

## [0.1.54](https://github.com/fohte/armyknife/compare/v0.1.53...v0.1.54) (2026-01-29)


### Bug Fixes

* **cc/hook:** escape tmux path in notification click action ([#179](https://github.com/fohte/armyknife/issues/179)) ([a645e4b](https://github.com/fohte/armyknife/commit/a645e4bbebbc1551b72411ce52037fac21e600e9))

## [0.1.53](https://github.com/fohte/armyknife/compare/v0.1.52...v0.1.53) (2026-01-29)


### Features

* **cc/hook:** show tool details in permission notifications using `PermissionRequest` hook ([#173](https://github.com/fohte/armyknife/issues/173)) ([0705451](https://github.com/fohte/armyknife/commit/0705451d7e5df0305a8d06eca4bf3f513b29c15f))

## [0.1.52](https://github.com/fohte/armyknife/compare/v0.1.51...v0.1.52) (2026-01-29)


### Bug Fixes

* **cc/hook:** use full path for tmux in notification click action ([#176](https://github.com/fohte/armyknife/issues/176)) ([f7e9a14](https://github.com/fohte/armyknife/commit/f7e9a1474cd2121944c2e52586803290cedc8fa7))

## [0.1.51](https://github.com/fohte/armyknife/compare/v0.1.50...v0.1.51) (2026-01-29)


### Bug Fixes

* **cc/store:** clean up sessions without TTY info ([#174](https://github.com/fohte/armyknife/issues/174)) ([0f1a684](https://github.com/fohte/armyknife/commit/0f1a684a9ea8870a1f2a21e3dc5bc57e446a52d1))

## [0.1.50](https://github.com/fohte/armyknife/compare/v0.1.49...v0.1.50) (2026-01-29)


### Features

* **wm/clean:** close tmux windows when deleting worktrees ([#171](https://github.com/fohte/armyknife/issues/171)) ([a16c933](https://github.com/fohte/armyknife/commit/a16c933dfd9fc4f5bcf398bd66dcf4b5b84a4955))

## [0.1.49](https://github.com/fohte/armyknife/compare/v0.1.48...v0.1.49) (2026-01-29)


### Bug Fixes

* **cc/hook:** prevent session file corruption from concurrent writes ([#169](https://github.com/fohte/armyknife/issues/169)) ([c1c2d05](https://github.com/fohte/armyknife/commit/c1c2d058597b79eb4b42e43389381f4905c54af0))

## [0.1.48](https://github.com/fohte/armyknife/compare/v0.1.47...v0.1.48) (2026-01-29)


### Bug Fixes

* **ai/review:** correct Devin bot login to match GitHub GraphQL API response ([#166](https://github.com/fohte/armyknife/issues/166)) ([0b3d15e](https://github.com/fohte/armyknife/commit/0b3d15ea4c4b7da2014f3e34bf7917667aa03719))

## [0.1.47](https://github.com/fohte/armyknife/compare/v0.1.46...v0.1.47) (2026-01-28)


### Features

* **cc/watch:** add search functionality with `/` key ([#163](https://github.com/fohte/armyknife/issues/163)) ([fbd402d](https://github.com/fohte/armyknife/commit/fbd402d8ea4462ca06915491f6cd2170d55ef159))

## [0.1.46](https://github.com/fohte/armyknife/compare/v0.1.45...v0.1.46) (2026-01-28)


### Features

* **cc/hook:** support log level control via `ARMYKNIFE_CC_HOOK_LOG` env var ([#162](https://github.com/fohte/armyknife/issues/162)) ([34d4ff3](https://github.com/fohte/armyknife/commit/34d4ff3378ede4c75a9e221fb1f6288fadd05a7b))

## [0.1.45](https://github.com/fohte/armyknife/compare/v0.1.44...v0.1.45) (2026-01-28)


### Features

* **cc/hook:** show session status, title, and last response in notifications ([#160](https://github.com/fohte/armyknife/issues/160)) ([9745afd](https://github.com/fohte/armyknife/commit/9745afde51783257d2c6835cbf4e03fb3c04fcad))

## [0.1.44](https://github.com/fohte/armyknife/compare/v0.1.43...v0.1.44) (2026-01-27)


### Features

* **ai/review:** support Devin Review and allow waiting for multiple reviewers ([#157](https://github.com/fohte/armyknife/issues/157)) ([a164158](https://github.com/fohte/armyknife/commit/a1641581423d74ab61f10cf4553516cb82f4e615))

## [0.1.43](https://github.com/fohte/armyknife/compare/v0.1.42...v0.1.43) (2026-01-27)


### Features

* **cc:** add notification support to `a cc hook` ([#154](https://github.com/fohte/armyknife/issues/154)) ([cf3b6f3](https://github.com/fohte/armyknife/commit/cf3b6f3a0bab2b90139e6d2b80b2e496b7534148))

## [0.1.42](https://github.com/fohte/armyknife/compare/v0.1.41...v0.1.42) (2026-01-27)


### Features

* **cc:** log raw stdin to file on JSON parse error ([#155](https://github.com/fohte/armyknife/issues/155)) ([4a63ca8](https://github.com/fohte/armyknife/commit/4a63ca82392c5f26b9beeff67fe22648523b6be9))

## [0.1.41](https://github.com/fohte/armyknife/compare/v0.1.40...v0.1.41) (2026-01-27)


### Features

* **cc:** display session status details in `a cc watch` ([#151](https://github.com/fohte/armyknife/issues/151)) ([28c73fe](https://github.com/fohte/armyknife/commit/28c73fedec4afc2ad7f35bd8d0bcdb64591ad65b))

## [0.1.40](https://github.com/fohte/armyknife/compare/v0.1.39...v0.1.40) (2026-01-27)


### Features

* **cc:** display Claude Code session titles in `a cc list` / `a cc watch` ([#148](https://github.com/fohte/armyknife/issues/148)) ([852b694](https://github.com/fohte/armyknife/commit/852b6945145b5975c1e704a7d34a1acfd3d7c6dd))

## [0.1.39](https://github.com/fohte/armyknife/compare/v0.1.38...v0.1.39) (2026-01-26)


### Bug Fixes

* **cc:** mark session as stopped on `idle_prompt` notification ([#146](https://github.com/fohte/armyknife/issues/146)) ([efc57f2](https://github.com/fohte/armyknife/commit/efc57f26df958b4944dadddf281fe54f449d7ad8))

## [0.1.38](https://github.com/fohte/armyknife/compare/v0.1.37...v0.1.38) (2026-01-25)


### Features

* **cc:** add `a cc watch` command for TUI-based session monitoring ([#144](https://github.com/fohte/armyknife/issues/144)) ([6e42df6](https://github.com/fohte/armyknife/commit/6e42df65e9a91520deb087ef6cdb32992e9fb57a))

## [0.1.37](https://github.com/fohte/armyknife/compare/v0.1.36...v0.1.37) (2026-01-25)


### Features

* **cc:** add `a cc focus` command ([#142](https://github.com/fohte/armyknife/issues/142)) ([1aa481a](https://github.com/fohte/armyknife/commit/1aa481a7fa85e37caee53b2935f75ba0d0864b62))

## [0.1.36](https://github.com/fohte/armyknife/compare/v0.1.35...v0.1.36) (2026-01-25)


### Features

* **cc:** add Claude Code session monitoring ([#137](https://github.com/fohte/armyknife/issues/137)) ([e51bc46](https://github.com/fohte/armyknife/commit/e51bc469072bfd3fb093d7b1daa95d352f99a02e))

## [0.1.35](https://github.com/fohte/armyknife/compare/v0.1.34...v0.1.35) (2026-01-25)


### Bug Fixes

* **wm/delete:** delete branch when running from within worktree ([#138](https://github.com/fohte/armyknife/issues/138)) ([360ce2b](https://github.com/fohte/armyknife/commit/360ce2b80a9b66253668c3fd94c7d60142b6f064))

## [0.1.34](https://github.com/fohte/armyknife/compare/v0.1.33...v0.1.34) (2026-01-24)


### Features

* **ai/pr-draft:** support updating existing PR on submit ([#133](https://github.com/fohte/armyknife/issues/133)) ([2ce9d82](https://github.com/fohte/armyknife/commit/2ce9d82db677c1b0128f34a90a98f957f33f2ee3))

## [0.1.33](https://github.com/fohte/armyknife/compare/v0.1.32...v0.1.33) (2026-01-24)


### Bug Fixes

* **tmux:** remove external `tmux-name` command dependency ([#129](https://github.com/fohte/armyknife/issues/129)) ([9cdde04](https://github.com/fohte/armyknife/commit/9cdde04ebbfe99c8baad16045f2a5b56a52db8c9))

## [0.1.32](https://github.com/fohte/armyknife/compare/v0.1.31...v0.1.32) (2026-01-24)


### Features

* **gh/issue-agent:** show local changes diff on `pull` ([#127](https://github.com/fohte/armyknife/issues/127)) ([89e45b1](https://github.com/fohte/armyknife/commit/89e45b1b5501c1e8933989ef86bb78ff3cc2fd8b))

## [0.1.31](https://github.com/fohte/armyknife/compare/v0.1.30...v0.1.31) (2026-01-23)


### Bug Fixes

* **gh/issue-agent:** prevent false positive change detection for comments with whitespace differences ([#124](https://github.com/fohte/armyknife/issues/124)) ([23860eb](https://github.com/fohte/armyknife/commit/23860eb87b8d7303830ba5bda75952c5766add35))

## [0.1.30](https://github.com/fohte/armyknife/compare/v0.1.29...v0.1.30) (2026-01-22)


### Dependencies

* update rust crate chrono to v0.4.43 ([#117](https://github.com/fohte/armyknife/issues/117)) ([f1ae6e7](https://github.com/fohte/armyknife/commit/f1ae6e798de262603947ba3499af83a3dda50da6))

## [0.1.29](https://github.com/fohte/armyknife/compare/v0.1.28...v0.1.29) (2026-01-22)


### Bug Fixes

* **update:** skip confirmation prompt ([#113](https://github.com/fohte/armyknife/issues/113)) ([4845753](https://github.com/fohte/armyknife/commit/4845753c9473ddab242fd312028540f5871e4c48))

## [0.1.28](https://github.com/fohte/armyknife/compare/v0.1.27...v0.1.28) (2026-01-22)


### Features

* **ai:** add `a ai draft` command ([#111](https://github.com/fohte/armyknife/issues/111)) ([a8d07e9](https://github.com/fohte/armyknife/commit/a8d07e9301b4b5343c3ac39150891d8a9035ebbf))

## [0.1.27](https://github.com/fohte/armyknife/compare/v0.1.26...v0.1.27) (2026-01-21)


### Bug Fixes

* **wm:** improve `wm new` output for clarity ([#105](https://github.com/fohte/armyknife/issues/105)) ([dbda156](https://github.com/fohte/armyknife/commit/dbda1561948b055196a931c0a22dcccce137c0d6))

## [0.1.26](https://github.com/fohte/armyknife/compare/v0.1.25...v0.1.26) (2026-01-20)


### Bug Fixes

* **gh/issue-agent:** retrieve issue comments correctly from GitHub API ([#103](https://github.com/fohte/armyknife/issues/103)) ([3acf53b](https://github.com/fohte/armyknife/commit/3acf53b02747095b09cfdfe329aeef2cf481b005))

## [0.1.25](https://github.com/fohte/armyknife/compare/v0.1.24...v0.1.25) (2026-01-19)


### Features

* **gh:** add `a gh issue-agent` command ([#80](https://github.com/fohte/armyknife/issues/80)) ([a80b9ee](https://github.com/fohte/armyknife/commit/a80b9ee9c99102c8817bb54a0feab2d4306c4ae5))

## [0.1.24](https://github.com/fohte/armyknife/compare/v0.1.23...v0.1.24) (2026-01-15)


### Bug Fixes

* **ci:** merge release-please PR directly when already in clean status ([#83](https://github.com/fohte/armyknife/issues/83)) ([196a892](https://github.com/fohte/armyknife/commit/196a89249e81c3054be47ef4f33eb55dc563d4fa))

## [0.1.23](https://github.com/fohte/armyknife/compare/v0.1.22...v0.1.23) (2026-01-15)


### Bug Fixes

* **wm:** use cache directory instead of state directory for macOS compatibility ([#81](https://github.com/fohte/armyknife/issues/81)) ([fb08e18](https://github.com/fohte/armyknife/commit/fb08e1847b3afdbf78066bd49680a86d29cf2c0b))

## [0.1.22](https://github.com/fohte/armyknife/compare/v0.1.21...v0.1.22) (2026-01-14)


### Features

* **ai:** add command to wait for Gemini Code Assist review ([#77](https://github.com/fohte/armyknife/issues/77)) ([058ff99](https://github.com/fohte/armyknife/commit/058ff996fb7a71dfe335a740872786e8956a4f2b))

## [0.1.21](https://github.com/fohte/armyknife/compare/v0.1.20...v0.1.21) (2026-01-14)


### Features

* **wm:** support editor input for prompt when `wm new` is invoked without arguments ([#75](https://github.com/fohte/armyknife/issues/75)) ([b16aea9](https://github.com/fohte/armyknife/commit/b16aea92b6bfa049afb018d9889f46f67d2eccc1))

## [0.1.20](https://github.com/fohte/armyknife/compare/v0.1.19...v0.1.20) (2026-01-13)


### Features

* **name-branch:** improve UX ([#73](https://github.com/fohte/armyknife/issues/73)) ([c403059](https://github.com/fohte/armyknife/commit/c403059f616cb0cb931b2dd858217900d431f3d3))

## [0.1.19](https://github.com/fohte/armyknife/compare/v0.1.18...v0.1.19) (2026-01-13)


### Dependencies

* update rust crate clap_complete to v4.5.64 ([#71](https://github.com/fohte/armyknife/issues/71)) ([82daa9f](https://github.com/fohte/armyknife/commit/82daa9fec59915f5704ed4f7287f11f4705a1719))

## [0.1.18](https://github.com/fohte/armyknife/compare/v0.1.17...v0.1.18) (2026-01-13)


### Features

* **cli:** support shell completion ([#69](https://github.com/fohte/armyknife/issues/69)) ([7898861](https://github.com/fohte/armyknife/commit/78988618373f108368c806e76d1ed96a72910cb9))

## [0.1.17](https://github.com/fohte/armyknife/compare/v0.1.16...v0.1.17) (2026-01-13)


### Bug Fixes

* **update:** prevent panic from nested tokio runtime ([#66](https://github.com/fohte/armyknife/issues/66)) ([0dadbb6](https://github.com/fohte/armyknife/commit/0dadbb601c6c1da04ad159f826244e58076d1283))

## [0.1.16](https://github.com/fohte/armyknife/compare/v0.1.15...v0.1.16) (2026-01-13)


### Bug Fixes

* **wm:** include macOS system gitconfig paths in credential helper lookup ([#63](https://github.com/fohte/armyknife/issues/63)) ([4545eb4](https://github.com/fohte/armyknife/commit/4545eb4b779ebba74cee91b118ecd4c97ad9dd9d))

## [0.1.15](https://github.com/fohte/armyknife/compare/v0.1.14...v0.1.15) (2026-01-13)


### Bug Fixes

* **wm:** support authentication for HTTPS private repositories ([#61](https://github.com/fohte/armyknife/issues/61)) ([e24fa1a](https://github.com/fohte/armyknife/commit/e24fa1aae1e1e916679553fde1c01af33c7eb9e5))

## [0.1.14](https://github.com/fohte/armyknife/compare/v0.1.13...v0.1.14) (2026-01-12)


### Features

* **wm:** support auto-generating branch name from prompt in `wm new` ([#59](https://github.com/fohte/armyknife/issues/59)) ([8813f9d](https://github.com/fohte/armyknife/commit/8813f9db08ad9e8feb5186ebee6ae650370a4923))

## [0.1.13](https://github.com/fohte/armyknife/compare/v0.1.12...v0.1.13) (2026-01-12)


### Bug Fixes

* **github:** show detailed API error messages and auto-detect default branch ([#57](https://github.com/fohte/armyknife/issues/57)) ([9e61a8a](https://github.com/fohte/armyknife/commit/9e61a8a24025dacd89b09129054ba00ab262e91a))

## [0.1.12](https://github.com/fohte/armyknife/compare/v0.1.11...v0.1.12) (2026-01-12)


### Bug Fixes

* **wm:** prevent nested Tokio runtime ([#55](https://github.com/fohte/armyknife/issues/55)) ([dab4f28](https://github.com/fohte/armyknife/commit/dab4f286d15928612d8ab0c6d4f0aee89ed695f7))

## [0.1.11](https://github.com/fohte/armyknife/compare/v0.1.10...v0.1.11) (2026-01-12)


### Features

* **gh:** add `a gh check-pr-review` command ([#37](https://github.com/fohte/armyknife/issues/37)) ([73ea430](https://github.com/fohte/armyknife/commit/73ea4308c860b7ce29cf27f49579d8b5da90b2fa))

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
