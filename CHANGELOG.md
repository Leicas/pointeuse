## [1.4.5](https://github.com/Leicas/pointeuse/compare/v1.4.4...v1.4.5) (2026-08-28)


### Bug Fixes

* unbreak Android release build, green CI lint, silence desktop notification ACL error ([#8](https://github.com/Leicas/pointeuse/issues/8)) ([cb4f4d0](https://github.com/Leicas/pointeuse/commit/cb4f4d00d9741b687d959efc0377399cc5f0822a))

## [1.4.4](https://github.com/Leicas/pointeuse/compare/v1.4.3...v1.4.4) (2026-08-28)


### Bug Fixes

* **android:** stop "keeps stopping" crashes + add Sentry crash reporting ([#7](https://github.com/Leicas/pointeuse/issues/7)) ([7848829](https://github.com/Leicas/pointeuse/commit/7848829164546f801d0e38af4c7de81e91f7f206))

## [1.4.3](https://github.com/Leicas/pointeuse/compare/v1.4.2...v1.4.3) (2026-08-27)


### Bug Fixes

* **sync:** back off background polls on consecutive network failures ([092c472](https://github.com/Leicas/pointeuse/commit/092c472bc08ccab3462d4ec6624eb65f6e9c25b8))

## [1.4.2](https://github.com/Leicas/pointeuse/compare/v1.4.1...v1.4.2) (2026-08-26)


### Bug Fixes

* **android:** restore HTTPS (reqwest 0.13 TLS panic) + timeouts, error chains, warm-process guard ([#6](https://github.com/Leicas/pointeuse/issues/6)) ([c6bb4da](https://github.com/Leicas/pointeuse/commit/c6bb4daba57b67b945e6832b241dccbc2c2ebf42))

## [1.4.1](https://github.com/Leicas/pointeuse/compare/v1.4.0...v1.4.1) (2026-08-26)


### Bug Fixes

* **android:** drop USE_EXACT_ALARM permission for Play policy compliance ([#4](https://github.com/Leicas/pointeuse/issues/4)) ([1271108](https://github.com/Leicas/pointeuse/commit/1271108bdd1fe30613acfbe97b65be7f6870189e))

# [1.4.0](https://github.com/Leicas/pointeuse/compare/v1.3.0...v1.4.0) (2026-08-04)


### Features

* **sync:** share the running timer across devices through Odoo ([6778c2b](https://github.com/Leicas/pointeuse/commit/6778c2b8cd46cb47511b943f42991f3605f1e760)), closes [#PTZ1](https://github.com/Leicas/pointeuse/issues/PTZ1)

# [1.3.0](https://github.com/Leicas/pointeuse/compare/v1.2.0...v1.3.0) (2026-07-30)


### Features

* **timesheet:** add manual entry composer with create, edit and delete ([874796c](https://github.com/Leicas/pointeuse/commit/874796c1a7f98a59de2bf3d7ebb2d99a844f539f))

# [1.2.0](https://github.com/Leicas/pointeuse/compare/v1.1.0...v1.2.0) (2026-06-13)


### Bug Fixes

* **updater:** avoid nested-runtime panic when installing an update ([cf3171e](https://github.com/Leicas/pointeuse/commit/cf3171e9b128cd8b3515ced144221f33d55f6967))


### Features

* **dashboard:** redesign task list + creation UX ([72ab4b9](https://github.com/Leicas/pointeuse/commit/72ab4b93c6959e0eb37e4600fa1c86d6ff94a7ea))

# [1.1.0](https://github.com/Leicas/pointeuse/compare/v1.0.1...v1.1.0) (2026-06-13)


### Bug Fixes

* **android:** add WorkManager dependency to app module in post-init patch ([0bb9875](https://github.com/Leicas/pointeuse/commit/0bb9875e5ea11635b633ca87a2c545ee78b4eae1))


### Features

* **branding:** replace H mark with a clock + checkmark logo ([4311fd7](https://github.com/Leicas/pointeuse/commit/4311fd730f537c8c542ad17da926a775a2e7503e))

## [1.0.1](https://github.com/Leicas/pointeuse/compare/v1.0.0...v1.0.1) (2026-06-13)


### Bug Fixes

* **ci:** satisfy clippy 1.96 and stop bundling gen_icon on macOS ([e353930](https://github.com/Leicas/pointeuse/commit/e3539300de95724196dd508d449e82a12435abfa))

# 1.0.0 (2026-06-12)


### Features

* initial release of Pointeuse ([268ab11](https://github.com/Leicas/pointeuse/commit/268ab11cc3850bae65a6961a4cdb45ea74b426ad))
