# Third-party notices

TuneWeave is an independent Rust implementation informed by the public protocol
research and implementations listed below. Source snapshots are recorded so
future ports can be audited precisely.

## NeteaseCloudMusicApiEnhanced/api-enhanced

- Source: https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced
- Reviewed commit: `41bd6d82ce3b494d6375a784f5af391340ed9c1b`
- License: MIT
- Used for: NetEase Cloud Music request protocols, endpoint behavior, response
  normalization, and authentication flow research.

## MOPELotus/Lotus-ReFactor

- Source: https://github.com/MOPELotus/Lotus-ReFactor
- Reviewed commit: `004bbff438bc811f0f28a9ddf4181e8b77a510ba`
- License: Lotus-ReFactor Source-Available Proprietary License
- Used for: NetEase Music Partner request logic and implementation details.
- Authorization: MOPELotus states that they contributed 100% of
  Lotus-ReFactor and explicitly authorizes TuneWeave to reference and reuse its
  logic and implementation. The public Lotus-ReFactor license is still recorded
  here accurately for third-party readers.

## L-1124/QQMusicApi

- Source: https://github.com/L-1124/QQMusicApi
- Reviewed commit: `1b0aae0db3ee6876b3a77b8d1ce3057b4b3c9cd5`
- License: GNU General Public License v3.0 or later
- Used for: QQ Music CGI request, authentication, catalog, playlist, lyric,
  media MID, file naming, VKey, and CDN behavior research.

QQMusicApi remains under GPL-3.0-or-later. TuneWeave does not copy, translate,
link, bundle, or redistribute its source code; the Rust implementation is an
independent implementation of observed request and response behavior.

## MakcRe/KuGouMusicApi

- Source: https://github.com/MakcRe/KuGouMusicApi
- Reviewed commit: `283f1e97b110726b208a64b486a657c0fc0a6126`
- License: MIT
- Used for: KuGou request signing, device identity, authentication, catalog,
  lyric, playlist, and media URL behavior research.

## Domdkw/miguMusic-api-enhanced

- Source: https://github.com/Domdkw/miguMusic-api-enhanced
- Reviewed commit: `14c55ffbbbd1a90afe5e6ac45425f7b7988730bd`
- License: Apache License 2.0
- Used for: Migu catalog, login, PACM token, resource identity, entitlement,
  and media URL behavior research.

## qyhqiu/kuwoMusicApi

- Source: https://github.com/qyhqiu/kuwoMusicApi
- Reviewed commit: `e8e720b90b4d7e3052078a3380906f2b3349e388`
- License: Apache License 2.0. The README does not declare an alternative;
  `package.json` contains stale ISC metadata and is not treated as overriding
  the root license. Endpoint usability will still be revalidated before porting.
- Used for: preliminary Kuwo endpoint inventory only.

## MOPELotus/BBDown

- Source: https://github.com/MOPELotus/BBDown
- Reviewed commit: `259a5558cee0a349a7ebb60bd31e40c88e5bc1ed`
- License: MIT
- Used for: Bilibili identifier parsing, metadata, multipart video, DASH audio
  and video track, authentication, and request header behavior research.

## bilibili-plugins/bilibili-api-collect

- Source: https://github.com/bilibili-plugins/bilibili-api-collect
- Reviewed commit: `cfc5fddcc8a94b74d91970bb5b4eaeb349addc47`
- License: Creative Commons Attribution-NonCommercial 4.0 International
- Used for: Bilibili public protocol documentation research, including account,
  user-space collection/season, favorites-folder, catalog, and write behavior.

The documentation remains under CC BY-NC 4.0. TuneWeave does not copy, bundle,
or redistribute its text or source; the Rust implementation independently
implements the documented request and response protocol.

## TuneWeave license texts

TuneWeave is available under either MIT or Apache-2.0 at the user's option.
The choice is summarized in `LICENSE`; complete texts are in `LICENSE-MIT` and
`LICENSE-APACHE`. Required third-party notices are retained under `licenses/`.
