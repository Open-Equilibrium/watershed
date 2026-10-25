# Third-Party Notices

Watershed-authored files remain licensed under `AGPL-3.0-only` unless otherwise stated. Vendored third-party material retains its own license.

## OpenClaw Agent Skills

- Purpose: vendored development/repository tooling; not Watershed product runtime.
- License: MIT.
- License text is bundled with the vendored material.
- Copyright notice: `Copyright (c) 2026 openclaw`.
- Upstream/source: `openclaw/agent-skills`.

## Cargokit

- Purpose: vendored build glue that compiles the Liquid Rust bridge into the Flutter app; part of the flutter_rust_bridge 2.13.0 template (ADR-0186).
- License: MIT.
- License text is bundled with the vendored material in `liquid/app/rust_builder/cargokit/LICENSE`.
- Modified: `build_pod.sh` no longer prints the build environment, which can hold credentials.
- Copyright notice: `Copyright 2022 Matej Knopp`.
- Upstream/source: `irondash/cargokit`.

## Lucide

- Purpose: icon font bundled into the Liquid app (ADR-0198); `lucide-static` 1.61.0, `font/lucide.ttf`.
- License: ISC; icons derived from Feather are MIT.
- License text is bundled with the vendored material in `liquid/app/assets/fonts/LICENSE-lucide` and shipped as an app asset.
- Copyright notice: `Copyright (c) 2026 Lucide Icons and Contributors`; `Copyright (c) 2013-present Cole Bemis`.
- Upstream/source: `lucide-icons/lucide`.
