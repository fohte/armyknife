# Changelog

## [0.1.99](https://github.com/fohte/armyknife/compare/v0.1.98...v0.1.99) (2026-02-17)


### Features

* **cc:** add status filter to `cc watch` ([#287](https://github.com/fohte/armyknife/issues/287)) ([50ce6f2](https://github.com/fohte/armyknife/commit/50ce6f253127489e00cc92e321723bcd3a5b831c))

## [0.1.98](https://github.com/fohte/armyknife/compare/v0.1.97...v0.1.98) (2026-02-17)


### Features

* **cc/tui:** fade session colors based on time since last update ([#286](https://github.com/fohte/armyknife/issues/286)) ([39e5347](https://github.com/fohte/armyknife/commit/39e534794e547407a79ef61db9a521302befd519))

## [0.1.97](https://github.com/fohte/armyknife/compare/v0.1.96...v0.1.97) (2026-02-16)


### Features

* **wm:** support `post-worktree-create` hook for `wm new` ([#283](https://github.com/fohte/armyknife/issues/283)) ([2c39ef4](https://github.com/fohte/armyknife/commit/2c39ef4daa77d7ba40a16b5a2b3382191ecfd340))

## [0.1.96](https://github.com/fohte/armyknife/compare/v0.1.95...v0.1.96) (2026-02-15)


### Features

* **wm:** add `--agent` flag to `wm new` for delegation context injection ([#279](https://github.com/fohte/armyknife/issues/279)) ([8a6e6b9](https://github.com/fohte/armyknife/commit/8a6e6b9071863f9c4a9652a8a892efa5e31c0734))

## [0.1.95](https://github.com/fohte/armyknife/compare/v0.1.94...v0.1.95) (2026-02-14)


### Features

* **wm:** support cleaning worktrees across all repositories with `--all` flag ([#275](https://github.com/fohte/armyknife/issues/275)) ([03539ae](https://github.com/fohte/armyknife/commit/03539aef08e5418fd86700276eff98350bd9e290))

## [0.1.94](https://github.com/fohte/armyknife/compare/v0.1.93...v0.1.94) (2026-02-12)


### Dependencies

* update rust crate anyhow to v1.0.101 ([#272](https://github.com/fohte/armyknife/issues/272)) ([ebb5a91](https://github.com/fohte/armyknife/commit/ebb5a919e34e68ce3159ffb9d320ac534a3276e5))

## [0.1.93](https://github.com/fohte/armyknife/compare/v0.1.92...v0.1.93) (2026-02-10)


### Dependencies

* update rust crate clap to v4.5.57 ([#269](https://github.com/fohte/armyknife/issues/269)) ([829a7f3](https://github.com/fohte/armyknife/commit/829a7f3c17bb3c378ce58b04ead9ece1dcd171b2))

## [0.1.92](https://github.com/fohte/armyknife/compare/v0.1.91...v0.1.92) (2026-02-10)


### Dependencies

* update rust crate regex to v1.12.3 ([#267](https://github.com/fohte/armyknife/issues/267)) ([3481542](https://github.com/fohte/armyknife/commit/34815423c23052b590dd2ba6020747ad6d34c184))

## [0.1.91](https://github.com/fohte/armyknife/compare/v0.1.90...v0.1.91) (2026-02-10)


### Features

* add `--version` flag ([#3](https://github.com/fohte/armyknife/issues/3)) ([ee08fe9](https://github.com/fohte/armyknife/commit/ee08fe9e6dbef96b1ae05d164044e22cbcfc1806))
* add automatic self-update ([#14](https://github.com/fohte/armyknife/issues/14)) ([43b650f](https://github.com/fohte/armyknife/commit/43b650fc9b19fe2198c4ab81c8159a67c98689c6))
* **ai/pr-draft:** support updating existing PR on submit ([#133](https://github.com/fohte/armyknife/issues/133)) ([2ce9d82](https://github.com/fohte/armyknife/commit/2ce9d82db677c1b0128f34a90a98f957f33f2ee3))
* **ai/review:** support Devin Review and allow waiting for multiple reviewers ([#157](https://github.com/fohte/armyknife/issues/157)) ([a164158](https://github.com/fohte/armyknife/commit/a1641581423d74ab61f10cf4553516cb82f4e615))
* **ai:** add `a ai draft` command ([#111](https://github.com/fohte/armyknife/issues/111)) ([a8d07e9](https://github.com/fohte/armyknife/commit/a8d07e9301b4b5343c3ac39150891d8a9035ebbf))
* **ai:** add `a ai pr-draft` command ([#20](https://github.com/fohte/armyknife/issues/20)) ([6badeea](https://github.com/fohte/armyknife/commit/6badeeabe5f16fe3d050f619d5ac371dca12dd8c))
* **ai:** add command to wait for Gemini Code Assist review ([#77](https://github.com/fohte/armyknife/issues/77)) ([058ff99](https://github.com/fohte/armyknife/commit/058ff996fb7a71dfe335a740872786e8956a4f2b))
* **cc/hook:** show session status, title, and last response in notifications ([#160](https://github.com/fohte/armyknife/issues/160)) ([9745afd](https://github.com/fohte/armyknife/commit/9745afde51783257d2c6835cbf4e03fb3c04fcad))
* **cc/hook:** show tool details in permission notifications using `PermissionRequest` hook ([#173](https://github.com/fohte/armyknife/issues/173)) ([0705451](https://github.com/fohte/armyknife/commit/0705451d7e5df0305a8d06eca4bf3f513b29c15f))
* **cc/hook:** support log level control via `ARMYKNIFE_CC_HOOK_LOG` env var ([#162](https://github.com/fohte/armyknife/issues/162)) ([34d4ff3](https://github.com/fohte/armyknife/commit/34d4ff3378ede4c75a9e221fb1f6288fadd05a7b))
* **cc/list:** support tmux status bar session status display ([#247](https://github.com/fohte/armyknife/issues/247)) ([1366ddf](https://github.com/fohte/armyknife/commit/1366ddfea744aef142e34d4b77916646300d6cbd))
* **cc/watch:** add search functionality with `/` key ([#163](https://github.com/fohte/armyknife/issues/163)) ([fbd402d](https://github.com/fohte/armyknife/commit/fbd402d8ea4462ca06915491f6cd2170d55ef159))
* **cc/watch:** highlight search query matches in session list ([#246](https://github.com/fohte/armyknife/issues/246)) ([ea47f5b](https://github.com/fohte/armyknife/commit/ea47f5b8ed95c6e47c2db74295080a3ad36d7785))
* **cc/watch:** preserve selected session across restarts ([#240](https://github.com/fohte/armyknife/issues/240)) ([84f6c3e](https://github.com/fohte/armyknife/commit/84f6c3ed1ce7e61fc86531c6aedabe4fe823e80b))
* **cc:** add `a cc focus` command ([#142](https://github.com/fohte/armyknife/issues/142)) ([1aa481a](https://github.com/fohte/armyknife/commit/1aa481a7fa85e37caee53b2935f75ba0d0864b62))
* **cc:** add `a cc watch` command for TUI-based session monitoring ([#144](https://github.com/fohte/armyknife/issues/144)) ([6e42df6](https://github.com/fohte/armyknife/commit/6e42df65e9a91520deb087ef6cdb32992e9fb57a))
* **cc:** add Claude Code session monitoring ([#137](https://github.com/fohte/armyknife/issues/137)) ([e51bc46](https://github.com/fohte/armyknife/commit/e51bc469072bfd3fb093d7b1daa95d352f99a02e))
* **cc:** add notification support to `a cc hook` ([#154](https://github.com/fohte/armyknife/issues/154)) ([cf3b6f3](https://github.com/fohte/armyknife/commit/cf3b6f3a0bab2b90139e6d2b80b2e496b7534148))
* **cc:** display Claude Code session titles in `a cc list` / `a cc watch` ([#148](https://github.com/fohte/armyknife/issues/148)) ([852b694](https://github.com/fohte/armyknife/commit/852b6945145b5975c1e704a7d34a1acfd3d7c6dd))
* **cc:** display session status details in `a cc watch` ([#151](https://github.com/fohte/armyknife/issues/151)) ([28c73fe](https://github.com/fohte/armyknife/commit/28c73fedec4afc2ad7f35bd8d0bcdb64591ad65b))
* **cc:** log raw stdin to file on JSON parse error ([#155](https://github.com/fohte/armyknife/issues/155)) ([4a63ca8](https://github.com/fohte/armyknife/commit/4a63ca82392c5f26b9beeff67fe22648523b6be9))
* **cc:** support session restoration after tmux resurrect ([#191](https://github.com/fohte/armyknife/issues/191)) ([6d4c76e](https://github.com/fohte/armyknife/commit/6d4c76e70959d47432208eb5abc113f731947147))
* **cc:** support session resumption using tmux user option ([#215](https://github.com/fohte/armyknife/issues/215)) ([202cf67](https://github.com/fohte/armyknife/commit/202cf672e0f02c2180f9a3b5ddc13f9f45d548e6))
* **cli:** support shell completion ([#69](https://github.com/fohte/armyknife/issues/69)) ([7898861](https://github.com/fohte/armyknife/commit/78988618373f108368c806e76d1ed96a72910cb9))
* **gh/issue-agent:** add diff command and colored diff output ([#184](https://github.com/fohte/armyknife/issues/184)) ([dd5a6cf](https://github.com/fohte/armyknife/commit/dd5a6cf7a9b7238fffa7f4962c9c0a99d24ca96e))
* **gh/issue-agent:** add init subcommand for boilerplate generation ([#188](https://github.com/fohte/armyknife/issues/188)) ([f9489e7](https://github.com/fohte/armyknife/commit/f9489e77087eab60386a5de6d6752172ce32ab8c))
* **gh/issue-agent:** display timeline events in view command ([#189](https://github.com/fohte/armyknife/issues/189)) ([fd8a75d](https://github.com/fohte/armyknife/commit/fd8a75d59acda77b838c5cd4a28bcc42ee59022c))
* **gh/issue-agent:** implement field-level conflict detection for push ([#218](https://github.com/fohte/armyknife/issues/218)) ([27dbe17](https://github.com/fohte/armyknife/commit/27dbe172e9e084760ad9b3e1eb73699e0e457499))
* **gh/issue-agent:** manage title in frontmatter instead of body h1 ([#201](https://github.com/fohte/armyknife/issues/201)) ([c791583](https://github.com/fohte/armyknife/commit/c7915835908346c9839a7b94d955608374491562))
* **gh/issue-agent:** show local changes diff on `pull` ([#127](https://github.com/fohte/armyknife/issues/127)) ([89e45b1](https://github.com/fohte/armyknife/commit/89e45b1b5501c1e8933989ef86bb78ff3cc2fd8b))
* **gh/issue-agent:** support new issue creation in push command ([#186](https://github.com/fohte/armyknife/issues/186)) ([1ceb85d](https://github.com/fohte/armyknife/commit/1ceb85d3627269daf1c5c5531b7d9677bd3d2ac8))
* **gh/issue-agent:** support repository issue templates in `init issue` ([#205](https://github.com/fohte/armyknife/issues/205)) ([ce5b146](https://github.com/fohte/armyknife/commit/ce5b146443a7fdd3a394f3c77f8a8cdfb7af9465))
* **gh:** add `a gh check-pr-review` command ([#37](https://github.com/fohte/armyknife/issues/37)) ([73ea430](https://github.com/fohte/armyknife/commit/73ea4308c860b7ce29cf27f49579d8b5da90b2fa))
* **gh:** add `a gh issue-agent` command ([#80](https://github.com/fohte/armyknife/issues/80)) ([a80b9ee](https://github.com/fohte/armyknife/commit/a80b9ee9c99102c8817bb54a0feab2d4306c4ae5))
* initialize Rust CLI project ([#1](https://github.com/fohte/armyknife/issues/1)) ([eb6978f](https://github.com/fohte/armyknife/commit/eb6978f6695f9fdca269e473f8d96b489800c0a6))
* **name-branch:** add command to auto-generate branch names from task descriptions ([#41](https://github.com/fohte/armyknife/issues/41)) ([4a2e36c](https://github.com/fohte/armyknife/commit/4a2e36cd8fc3318c8d16356959b5c36d0cb5dbb7))
* **name-branch:** improve UX ([#73](https://github.com/fohte/armyknife/issues/73)) ([c403059](https://github.com/fohte/armyknife/commit/c403059f616cb0cb931b2dd858217900d431f3d3))
* support user configuration via config file ([#253](https://github.com/fohte/armyknife/issues/253)) ([f05eb2b](https://github.com/fohte/armyknife/commit/f05eb2b76947aae17544ced91a72c06d5be8d949))
* **wm/clean:** close tmux windows when deleting worktrees ([#171](https://github.com/fohte/armyknife/issues/171)) ([a16c933](https://github.com/fohte/armyknife/commit/a16c933dfd9fc4f5bcf398bd66dcf4b5b84a4955))
* **wm:** add git worktree management command ([#36](https://github.com/fohte/armyknife/issues/36)) ([259a8f2](https://github.com/fohte/armyknife/commit/259a8f29cf19cf9be13ddebe2c35bbe25943bc0e))
* **wm:** improve wm clean output with table format ([#194](https://github.com/fohte/armyknife/issues/194)) ([437da86](https://github.com/fohte/armyknife/commit/437da86fae3911f0f108176050f8c13e5771ce7c))
* **wm:** support auto-generating branch name from prompt in `wm new` ([#59](https://github.com/fohte/armyknife/issues/59)) ([8813f9d](https://github.com/fohte/armyknife/commit/8813f9db08ad9e8feb5186ebee6ae650370a4923))
* **wm:** support editor input for prompt when `wm new` is invoked without arguments ([#75](https://github.com/fohte/armyknife/issues/75)) ([b16aea9](https://github.com/fohte/armyknife/commit/b16aea92b6bfa049afb018d9889f46f67d2eccc1))


### Bug Fixes

* **ai/pr-draft:** restore to focused pane at review command execution instead of source pane ([#42](https://github.com/fohte/armyknife/issues/42)) ([51c3a76](https://github.com/fohte/armyknife/commit/51c3a76c63bd5bb7d89a3012e93a5d3e609caf59))
* **ai/pr-draft:** use stable tmux IDs for window/pane restoration ([#35](https://github.com/fohte/armyknife/issues/35)) ([5c8eaae](https://github.com/fohte/armyknife/commit/5c8eaaeb53d4f602ffcae438b99f3d94944e4cd5))
* **ai/review:** correct Devin bot login to match GitHub GraphQL API response ([#166](https://github.com/fohte/armyknife/issues/166)) ([0b3d15e](https://github.com/fohte/armyknife/commit/0b3d15ea4c4b7da2014f3e34bf7917667aa03719))
* **ai:** fix window title and private repo detection in pr-draft ([#26](https://github.com/fohte/armyknife/issues/26)) ([fdd4760](https://github.com/fohte/armyknife/commit/fdd47601eebd2964b263a468c6cfab368ad11724))
* **cc/hook:** ensure stop hook notification shows the latest assistant response ([#248](https://github.com/fohte/armyknife/issues/248)) ([ad637d8](https://github.com/fohte/armyknife/commit/ad637d8c3cb6d5af74adf90234b29bfa5f2fd2b7))
* **cc/hook:** escape tmux path in notification click action ([#179](https://github.com/fohte/armyknife/issues/179)) ([a645e4b](https://github.com/fohte/armyknife/commit/a645e4bbebbc1551b72411ce52037fac21e600e9))
* **cc/hook:** prevent duplicate session creation on `claude -c` resume ([#228](https://github.com/fohte/armyknife/issues/228)) ([25c941f](https://github.com/fohte/armyknife/commit/25c941fd9485ee2e118f518711f6558d3f391c41))
* **cc/hook:** prevent session file corruption from concurrent writes ([#169](https://github.com/fohte/armyknife/issues/169)) ([c1c2d05](https://github.com/fohte/armyknife/commit/c1c2d058597b79eb4b42e43389381f4905c54af0))
* **cc/hook:** use full path for tmux in notification click action ([#176](https://github.com/fohte/armyknife/issues/176)) ([f7e9a14](https://github.com/fohte/armyknife/commit/f7e9a1474cd2121944c2e52586803290cedc8fa7))
* **cc/store:** clean up sessions without TTY info ([#174](https://github.com/fohte/armyknife/issues/174)) ([0f1a684](https://github.com/fohte/armyknife/commit/0f1a684a9ea8870a1f2a21e3dc5bc57e446a52d1))
* **cc/watch:** focus selected session directly from search mode ([#241](https://github.com/fohte/armyknife/issues/241)) ([416c46e](https://github.com/fohte/armyknife/commit/416c46e65ead623d82223dc83240e5b6ca8c365e))
* **cc/watch:** optimize idle performance ([#181](https://github.com/fohte/armyknife/issues/181)) ([73f4a02](https://github.com/fohte/armyknife/commit/73f4a02f49d7b5e33ecb659931638cfbd51a0d00))
* **cc/watch:** stabilize session list sort order during concurrent execution ([#230](https://github.com/fohte/armyknife/issues/230)) ([0fad137](https://github.com/fohte/armyknife/commit/0fad13710b037a85076d7375b836f75fd155576e))
* **cc/watch:** use pane_id only for tmux focus to handle window index drift ([#226](https://github.com/fohte/armyknife/issues/226)) ([1caf219](https://github.com/fohte/armyknife/commit/1caf21926321d8a35f96045c4d93e322dd58a29a))
* **cc/watch:** use tmux pane existence check for session lifecycle detection ([#187](https://github.com/fohte/armyknife/issues/187)) ([c481e73](https://github.com/fohte/armyknife/commit/c481e73dc3d9acc4f259b7144fe7195c80accad3))
* **cc:** allow focusing panes across different tmux sessions ([#236](https://github.com/fohte/armyknife/issues/236)) ([688c19d](https://github.com/fohte/armyknife/commit/688c19dea433ddd484e99ebe09c28b47e5212829))
* **cc:** mark session as stopped on `idle_prompt` notification ([#146](https://github.com/fohte/armyknife/issues/146)) ([efc57f2](https://github.com/fohte/armyknife/commit/efc57f26df958b4944dadddf281fe54f449d7ad8))
* **ci:** merge release-please PR directly when already in clean status ([#83](https://github.com/fohte/armyknife/issues/83)) ([196a892](https://github.com/fohte/armyknife/commit/196a89249e81c3054be47ef4f33eb55dc563d4fa))
* **deps:** pin rust crate clap to v4.5.53 ([#6](https://github.com/fohte/armyknife/issues/6)) ([957a246](https://github.com/fohte/armyknife/commit/957a2465fa5f7dd926139bfc7d85570d458233e1))
* **gh/issue-agent:** correct misleading message after `init issue` ([#199](https://github.com/fohte/armyknife/issues/199)) ([fe35754](https://github.com/fohte/armyknife/commit/fe3575430fa521ffd394a25ce0101f5582718a3d))
* **gh/issue-agent:** correct misleading message for new comments in `pull --force` ([#216](https://github.com/fohte/armyknife/issues/216)) ([54675e6](https://github.com/fohte/armyknife/commit/54675e6e4870c654b99d72c518580ce7823d968c))
* **gh/issue-agent:** prevent false positive change detection for comments with whitespace differences ([#124](https://github.com/fohte/armyknife/issues/124)) ([23860eb](https://github.com/fohte/armyknife/commit/23860eb87b8d7303830ba5bda75952c5766add35))
* **gh/issue-agent:** retrieve issue comments correctly from GitHub API ([#103](https://github.com/fohte/armyknife/issues/103)) ([3acf53b](https://github.com/fohte/armyknife/commit/3acf53b02747095b09cfdfe329aeef2cf481b005))
* **gh/issue-agent:** use correct GraphQL fields for conflict detection ([#222](https://github.com/fohte/armyknife/issues/222)) ([ecdb0a6](https://github.com/fohte/armyknife/commit/ecdb0a6848d6bda3dc9910401910f478c33772f9))
* **gh/issue-agent:** validate repository existence in init command ([#202](https://github.com/fohte/armyknife/issues/202)) ([83ccb44](https://github.com/fohte/armyknife/commit/83ccb448342ec746d04ebb38cb79befbf79c2ce3))
* **github:** show detailed API error messages and auto-detect default branch ([#57](https://github.com/fohte/armyknife/issues/57)) ([9e61a8a](https://github.com/fohte/armyknife/commit/9e61a8a24025dacd89b09129054ba00ab262e91a))
* **pr-draft:** prevent accidental overwrite of existing draft files ([#33](https://github.com/fohte/armyknife/issues/33)) ([8be82a1](https://github.com/fohte/armyknife/commit/8be82a11f0f565a4135bcd8f6c1869c27332e98e))
* resolve Ghostty permission dialog, window size, and tmux interference on macOS ([#264](https://github.com/fohte/armyknife/issues/264)) ([4da2212](https://github.com/fohte/armyknife/commit/4da22128b6c001540673a97f64275bd2645424ed))
* **review:** skip unable reviewers instead of failing the entire wait ([#262](https://github.com/fohte/armyknife/issues/262)) ([9b67983](https://github.com/fohte/armyknife/commit/9b67983ff874f11d9ee5381ea4c84516a94a3c72))
* **tmux:** remove external `tmux-name` command dependency ([#129](https://github.com/fohte/armyknife/issues/129)) ([9cdde04](https://github.com/fohte/armyknife/commit/9cdde04ebbfe99c8baad16045f2a5b56a52db8c9))
* **update:** fix tar archive extraction error in `a update` ([#29](https://github.com/fohte/armyknife/issues/29)) ([193f468](https://github.com/fohte/armyknife/commit/193f4683b130cc26184220c023c14ad5169998cf))
* **update:** prevent panic from nested tokio runtime ([#66](https://github.com/fohte/armyknife/issues/66)) ([0dadbb6](https://github.com/fohte/armyknife/commit/0dadbb601c6c1da04ad159f826244e58076d1283))
* **update:** skip confirmation prompt ([#113](https://github.com/fohte/armyknife/issues/113)) ([4845753](https://github.com/fohte/armyknife/commit/4845753c9473ddab242fd312028540f5871e4c48))
* **update:** support gzip decompression ([#31](https://github.com/fohte/armyknife/issues/31)) ([708b7a6](https://github.com/fohte/armyknife/commit/708b7a6dae1a1b7b608e4c8e53fe8982c44559a8))
* use XDG base directories instead of platform-specific paths on macOS ([#256](https://github.com/fohte/armyknife/issues/256)) ([3dc590c](https://github.com/fohte/armyknife/commit/3dc590cfb4fdf0427b5cbba8fcb33e682cbcfbc2))
* **wm/delete:** delete branch when running from within worktree ([#138](https://github.com/fohte/armyknife/issues/138)) ([360ce2b](https://github.com/fohte/armyknife/commit/360ce2b80a9b66253668c3fd94c7d60142b6f064))
* **wm:** improve `wm new` output for clarity ([#105](https://github.com/fohte/armyknife/issues/105)) ([dbda156](https://github.com/fohte/armyknife/commit/dbda1561948b055196a931c0a22dcccce137c0d6))
* **wm:** include macOS system gitconfig paths in credential helper lookup ([#63](https://github.com/fohte/armyknife/issues/63)) ([4545eb4](https://github.com/fohte/armyknife/commit/4545eb4b779ebba74cee91b118ecd4c97ad9dd9d))
* **wm:** prevent nested Tokio runtime ([#55](https://github.com/fohte/armyknife/issues/55)) ([dab4f28](https://github.com/fohte/armyknife/commit/dab4f286d15928612d8ab0c6d4f0aee89ed695f7))
* **wm:** support authentication for HTTPS private repositories ([#61](https://github.com/fohte/armyknife/issues/61)) ([e24fa1a](https://github.com/fohte/armyknife/commit/e24fa1aae1e1e916679553fde1c01af33c7eb9e5))
* **wm:** use cache directory instead of state directory for macOS compatibility ([#81](https://github.com/fohte/armyknife/issues/81)) ([fb08e18](https://github.com/fohte/armyknife/commit/fb08e1847b3afdbf78066bd49680a86d29cf2c0b))


### Performance Improvements

* **cc/watch:** improve performance during hook invocations ([#207](https://github.com/fohte/armyknife/issues/207)) ([bfcfbb4](https://github.com/fohte/armyknife/commit/bfcfbb47e3ff65336d5b0b7e60fccfaf34700792))
* **cc/watch:** resolve key input lag during rapid hook firing ([#231](https://github.com/fohte/armyknife/issues/231)) ([483426e](https://github.com/fohte/armyknife/commit/483426e352fdae30ee993aa2b22745fb35b77278))


### Dependencies

* update rust crate chrono to v0.4.43 ([#117](https://github.com/fohte/armyknife/issues/117)) ([f1ae6e7](https://github.com/fohte/armyknife/commit/f1ae6e798de262603947ba3499af83a3dda50da6))
* update rust crate clap to v4.5.55 ([#224](https://github.com/fohte/armyknife/issues/224)) ([1fa9f62](https://github.com/fohte/armyknife/commit/1fa9f62058e74e8291d019cf9c1a42db50643f3a))
* update rust crate clap to v4.5.56 ([#244](https://github.com/fohte/armyknife/issues/244)) ([3a4c202](https://github.com/fohte/armyknife/commit/3a4c202530d528e92109814bf48034481de69c58))
* update rust crate clap_complete to v4.5.64 ([#71](https://github.com/fohte/armyknife/issues/71)) ([82daa9f](https://github.com/fohte/armyknife/commit/82daa9fec59915f5704ed4f7287f11f4705a1719))
* update rust crate clap_complete to v4.5.65 ([#78](https://github.com/fohte/armyknife/issues/78)) ([258c7a5](https://github.com/fohte/armyknife/commit/258c7a590106c71ac57adaeb64e06dabd9e297d0))
* update rust crate git2 to v0.20.4 [security] ([#233](https://github.com/fohte/armyknife/issues/233)) ([d8a4e58](https://github.com/fohte/armyknife/commit/d8a4e58a7ad342342734e39b39984e176d0e7202))
* update rust crate http to v1.4.0 ([#121](https://github.com/fohte/armyknife/issues/121)) ([f36c784](https://github.com/fohte/armyknife/commit/f36c7848946c56f81fa76946248f2faf36703a0e))
* update rust crate http to v1.4.0 ([#210](https://github.com/fohte/armyknife/issues/210)) ([d475dff](https://github.com/fohte/armyknife/commit/d475dffe71e1b18c10015429954f82df7f8b84ed))
* update rust crate notify-rust to v4.12.0 ([#255](https://github.com/fohte/armyknife/issues/255)) ([4f5ceb8](https://github.com/fohte/armyknife/commit/4f5ceb8ece6ff0ffa9d6bff44767c947a4b4492d))

## [0.1.90](https://github.com/fohte/armyknife/compare/v0.1.89...v0.1.90) (2026-02-10)


### Bug Fixes

* resolve Ghostty permission dialog, window size, and tmux interference on macOS ([#264](https://github.com/fohte/armyknife/issues/264)) ([4da2212](https://github.com/fohte/armyknife/commit/4da22128b6c001540673a97f64275bd2645424ed))


### Dependencies

* update rust crate notify-rust to v4.12.0 ([#255](https://github.com/fohte/armyknife/issues/255)) ([4f5ceb8](https://github.com/fohte/armyknife/commit/4f5ceb8ece6ff0ffa9d6bff44767c947a4b4492d))

## [0.1.89](https://github.com/fohte/armyknife/compare/v0.1.88...v0.1.89) (2026-02-09)


### Bug Fixes

* **review:** skip unable reviewers instead of failing the entire wait ([#262](https://github.com/fohte/armyknife/issues/262)) ([9b67983](https://github.com/fohte/armyknife/commit/9b67983ff874f11d9ee5381ea4c84516a94a3c72))

## [0.1.88](https://github.com/fohte/armyknife/compare/v0.1.87...v0.1.88) (2026-02-09)


### Bug Fixes

* use XDG base directories instead of platform-specific paths on macOS ([#256](https://github.com/fohte/armyknife/issues/256)) ([3dc590c](https://github.com/fohte/armyknife/commit/3dc590cfb4fdf0427b5cbba8fcb33e682cbcfbc2))

## [0.1.87](https://github.com/fohte/armyknife/compare/v0.1.86...v0.1.87) (2026-02-09)


### Features

* support user configuration via config file ([#253](https://github.com/fohte/armyknife/issues/253)) ([f05eb2b](https://github.com/fohte/armyknife/commit/f05eb2b76947aae17544ced91a72c06d5be8d949))

## [0.1.86](https://github.com/fohte/armyknife/compare/v0.1.85...v0.1.86) (2026-02-07)


### Bug Fixes

* **cc/hook:** ensure stop hook notification shows the latest assistant response ([#248](https://github.com/fohte/armyknife/issues/248)) ([ad637d8](https://github.com/fohte/armyknife/commit/ad637d8c3cb6d5af74adf90234b29bfa5f2fd2b7))

## [0.1.85](https://github.com/fohte/armyknife/compare/v0.1.84...v0.1.85) (2026-02-07)


### Features

* **cc/list:** support tmux status bar session status display ([#247](https://github.com/fohte/armyknife/issues/247)) ([1366ddf](https://github.com/fohte/armyknife/commit/1366ddfea744aef142e34d4b77916646300d6cbd))

## [0.1.84](https://github.com/fohte/armyknife/compare/v0.1.83...v0.1.84) (2026-02-07)


### Features

* **cc/watch:** highlight search query matches in session list ([#246](https://github.com/fohte/armyknife/issues/246)) ([ea47f5b](https://github.com/fohte/armyknife/commit/ea47f5b8ed95c6e47c2db74295080a3ad36d7785))

## [0.1.83](https://github.com/fohte/armyknife/compare/v0.1.82...v0.1.83) (2026-02-05)


### Dependencies

* update rust crate clap to v4.5.56 ([#244](https://github.com/fohte/armyknife/issues/244)) ([3a4c202](https://github.com/fohte/armyknife/commit/3a4c202530d528e92109814bf48034481de69c58))

## [0.1.82](https://github.com/fohte/armyknife/compare/v0.1.81...v0.1.82) (2026-02-05)


### Bug Fixes

* **cc/watch:** focus selected session directly from search mode ([#241](https://github.com/fohte/armyknife/issues/241)) ([416c46e](https://github.com/fohte/armyknife/commit/416c46e65ead623d82223dc83240e5b6ca8c365e))

## [0.1.81](https://github.com/fohte/armyknife/compare/v0.1.80...v0.1.81) (2026-02-05)


### Features

* **cc/watch:** preserve selected session across restarts ([#240](https://github.com/fohte/armyknife/issues/240)) ([84f6c3e](https://github.com/fohte/armyknife/commit/84f6c3ed1ce7e61fc86531c6aedabe4fe823e80b))

## [0.1.80](https://github.com/fohte/armyknife/compare/v0.1.79...v0.1.80) (2026-02-05)


### Bug Fixes

* **cc:** allow focusing panes across different tmux sessions ([#236](https://github.com/fohte/armyknife/issues/236)) ([688c19d](https://github.com/fohte/armyknife/commit/688c19dea433ddd484e99ebe09c28b47e5212829))

## [0.1.79](https://github.com/fohte/armyknife/compare/v0.1.78...v0.1.79) (2026-02-05)


### Performance Improvements

* **cc/watch:** resolve key input lag during rapid hook firing ([#231](https://github.com/fohte/armyknife/issues/231)) ([483426e](https://github.com/fohte/armyknife/commit/483426e352fdae30ee993aa2b22745fb35b77278))

## [0.1.78](https://github.com/fohte/armyknife/compare/v0.1.77...v0.1.78) (2026-02-04)


### Dependencies

* update rust crate git2 to v0.20.4 [security] ([#233](https://github.com/fohte/armyknife/issues/233)) ([d8a4e58](https://github.com/fohte/armyknife/commit/d8a4e58a7ad342342734e39b39984e176d0e7202))

## [0.1.77](https://github.com/fohte/armyknife/compare/v0.1.76...v0.1.77) (2026-02-04)


### Bug Fixes

* **cc/watch:** stabilize session list sort order during concurrent execution ([#230](https://github.com/fohte/armyknife/issues/230)) ([0fad137](https://github.com/fohte/armyknife/commit/0fad13710b037a85076d7375b836f75fd155576e))

## [0.1.76](https://github.com/fohte/armyknife/compare/v0.1.75...v0.1.76) (2026-02-04)


### Bug Fixes

* **cc/hook:** prevent duplicate session creation on `claude -c` resume ([#228](https://github.com/fohte/armyknife/issues/228)) ([25c941f](https://github.com/fohte/armyknife/commit/25c941fd9485ee2e118f518711f6558d3f391c41))

## [0.1.75](https://github.com/fohte/armyknife/compare/v0.1.74...v0.1.75) (2026-02-04)


### Bug Fixes

* **cc/watch:** use pane_id only for tmux focus to handle window index drift ([#226](https://github.com/fohte/armyknife/issues/226)) ([1caf219](https://github.com/fohte/armyknife/commit/1caf21926321d8a35f96045c4d93e322dd58a29a))

## [0.1.74](https://github.com/fohte/armyknife/compare/v0.1.73...v0.1.74) (2026-02-03)


### Dependencies

* update rust crate clap to v4.5.55 ([#224](https://github.com/fohte/armyknife/issues/224)) ([1fa9f62](https://github.com/fohte/armyknife/commit/1fa9f62058e74e8291d019cf9c1a42db50643f3a))

## [0.1.73](https://github.com/fohte/armyknife/compare/v0.1.72...v0.1.73) (2026-02-03)


### Bug Fixes

* **gh/issue-agent:** use correct GraphQL fields for conflict detection ([#222](https://github.com/fohte/armyknife/issues/222)) ([ecdb0a6](https://github.com/fohte/armyknife/commit/ecdb0a6848d6bda3dc9910401910f478c33772f9))

## [0.1.72](https://github.com/fohte/armyknife/compare/v0.1.71...v0.1.72) (2026-02-02)


### Features

* **gh/issue-agent:** implement field-level conflict detection for push ([#218](https://github.com/fohte/armyknife/issues/218)) ([27dbe17](https://github.com/fohte/armyknife/commit/27dbe172e9e084760ad9b3e1eb73699e0e457499))

## [0.1.71](https://github.com/fohte/armyknife/compare/v0.1.70...v0.1.71) (2026-02-02)


### Features

* **cc:** support session resumption using tmux user option ([#215](https://github.com/fohte/armyknife/issues/215)) ([202cf67](https://github.com/fohte/armyknife/commit/202cf672e0f02c2180f9a3b5ddc13f9f45d548e6))

## [0.1.70](https://github.com/fohte/armyknife/compare/v0.1.69...v0.1.70) (2026-02-02)


### Bug Fixes

* **gh/issue-agent:** correct misleading message for new comments in `pull --force` ([#216](https://github.com/fohte/armyknife/issues/216)) ([54675e6](https://github.com/fohte/armyknife/commit/54675e6e4870c654b99d72c518580ce7823d968c))

## [0.1.69](https://github.com/fohte/armyknife/compare/v0.1.68...v0.1.69) (2026-02-02)


### Dependencies

* update rust crate http to v1.4.0 ([#210](https://github.com/fohte/armyknife/issues/210)) ([d475dff](https://github.com/fohte/armyknife/commit/d475dffe71e1b18c10015429954f82df7f8b84ed))

## [0.1.68](https://github.com/fohte/armyknife/compare/v0.1.67...v0.1.68) (2026-02-02)


### Dependencies

* update rust crate http to v1.4.0 ([#121](https://github.com/fohte/armyknife/issues/121)) ([f36c784](https://github.com/fohte/armyknife/commit/f36c7848946c56f81fa76946248f2faf36703a0e))

## [0.1.67](https://github.com/fohte/armyknife/compare/v0.1.66...v0.1.67) (2026-02-02)


### Performance Improvements

* **cc/watch:** improve performance during hook invocations ([#207](https://github.com/fohte/armyknife/issues/207)) ([bfcfbb4](https://github.com/fohte/armyknife/commit/bfcfbb47e3ff65336d5b0b7e60fccfaf34700792))

## [0.1.66](https://github.com/fohte/armyknife/compare/v0.1.65...v0.1.66) (2026-02-02)


### Features

* **gh/issue-agent:** support repository issue templates in `init issue` ([#205](https://github.com/fohte/armyknife/issues/205)) ([ce5b146](https://github.com/fohte/armyknife/commit/ce5b146443a7fdd3a394f3c77f8a8cdfb7af9465))

## [0.1.65](https://github.com/fohte/armyknife/compare/v0.1.64...v0.1.65) (2026-02-02)


### Bug Fixes

* **gh/issue-agent:** validate repository existence in init command ([#202](https://github.com/fohte/armyknife/issues/202)) ([83ccb44](https://github.com/fohte/armyknife/commit/83ccb448342ec746d04ebb38cb79befbf79c2ce3))

## [0.1.64](https://github.com/fohte/armyknife/compare/v0.1.63...v0.1.64) (2026-02-02)


### Features

* **gh/issue-agent:** manage title in frontmatter instead of body h1 ([#201](https://github.com/fohte/armyknife/issues/201)) ([c791583](https://github.com/fohte/armyknife/commit/c7915835908346c9839a7b94d955608374491562))

## [0.1.63](https://github.com/fohte/armyknife/compare/v0.1.62...v0.1.63) (2026-02-02)


### Bug Fixes

* **gh/issue-agent:** correct misleading message after `init issue` ([#199](https://github.com/fohte/armyknife/issues/199)) ([fe35754](https://github.com/fohte/armyknife/commit/fe3575430fa521ffd394a25ce0101f5582718a3d))

## [0.1.62](https://github.com/fohte/armyknife/compare/v0.1.61...v0.1.62) (2026-02-01)


### Features

* **gh/issue-agent:** support new issue creation in push command ([#186](https://github.com/fohte/armyknife/issues/186)) ([1ceb85d](https://github.com/fohte/armyknife/commit/1ceb85d3627269daf1c5c5531b7d9677bd3d2ac8))

## [0.1.61](https://github.com/fohte/armyknife/compare/v0.1.60...v0.1.61) (2026-02-01)


### Features

* **wm:** improve wm clean output with table format ([#194](https://github.com/fohte/armyknife/issues/194)) ([437da86](https://github.com/fohte/armyknife/commit/437da86fae3911f0f108176050f8c13e5771ce7c))

## [0.1.60](https://github.com/fohte/armyknife/compare/v0.1.59...v0.1.60) (2026-02-01)


### Features

* **cc:** support session restoration after tmux resurrect ([#191](https://github.com/fohte/armyknife/issues/191)) ([6d4c76e](https://github.com/fohte/armyknife/commit/6d4c76e70959d47432208eb5abc113f731947147))

## [0.1.59](https://github.com/fohte/armyknife/compare/v0.1.58...v0.1.59) (2026-02-01)


### Features

* **gh/issue-agent:** display timeline events in view command ([#189](https://github.com/fohte/armyknife/issues/189)) ([fd8a75d](https://github.com/fohte/armyknife/commit/fd8a75d59acda77b838c5cd4a28bcc42ee59022c))

## [0.1.58](https://github.com/fohte/armyknife/compare/v0.1.57...v0.1.58) (2026-02-01)


### Features

* **gh/issue-agent:** add init subcommand for boilerplate generation ([#188](https://github.com/fohte/armyknife/issues/188)) ([f9489e7](https://github.com/fohte/armyknife/commit/f9489e77087eab60386a5de6d6752172ce32ab8c))

## [0.1.57](https://github.com/fohte/armyknife/compare/v0.1.56...v0.1.57) (2026-02-01)


### Bug Fixes

* **cc/watch:** use tmux pane existence check for session lifecycle detection ([#187](https://github.com/fohte/armyknife/issues/187)) ([c481e73](https://github.com/fohte/armyknife/commit/c481e73dc3d9acc4f259b7144fe7195c80accad3))

## [0.1.56](https://github.com/fohte/armyknife/compare/v0.1.55...v0.1.56) (2026-02-01)


### Features

* **gh/issue-agent:** add diff command and colored diff output ([#184](https://github.com/fohte/armyknife/issues/184)) ([dd5a6cf](https://github.com/fohte/armyknife/commit/dd5a6cf7a9b7238fffa7f4962c9c0a99d24ca96e))

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
