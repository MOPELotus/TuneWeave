# Third-party notices

TuneWeave is an independent Rust implementation informed by the public protocol
research and implementations listed below. Source snapshots are recorded so
future ports can be audited precisely.

## NeteaseCloudMusicApiEnhanced/api-enhanced

- Source: https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced
- Reviewed commit: `63d89aa906f78c286a7f838258fa29220d7f41dd`
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
- Reviewed commit: `873255f2774361ac97366bd89a14b8ed9d230aae`
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
- Reviewed commit: `47d2edb7175cf2874882273ed14be0fdfe7db796`
- License: Apache License 2.0
- Used for: Migu catalog, login, PACM token, resource identity, entitlement,
  and media URL behavior research.

## qyhqiu/kuwoMusicApi

- Source: https://github.com/qyhqiu/kuwoMusicApi
- Reviewed commit: `e8e720b90b4d7e3052078a3380906f2b3349e388`
- License: Apache License 2.0. The README does not declare an alternative;
  `package.json` contains stale ISC metadata and is not treated as overriding
  the root license.
- Used for: historical Kuwo endpoint inventory, legacy response fields, and
  project-scope research. Current endpoint usability is determined from the
  official website and client rather than this snapshot.

## guohuiyuan/music-lib

- Source: https://github.com/guohuiyuan/music-lib
- Reviewed commit: `b299302e3163765d3efcc9df592700b41867c3d8`
- License: GNU Affero General Public License v3.0
- Used for: Kuwo lyric, album, playlist, and field-model research, plus Soda
  Music share, `track_v2`, lyric, catalog, entitlement, and session research.

TuneWeave does not copy, translate, link, bundle, or redistribute music-lib
source. Its AGPL-3.0 implementation is used only to identify protocol facts
that are independently validated against official services and reimplemented
under TuneWeave's architecture.

## UnblockNeteaseMusic/server

- Source: https://github.com/UnblockNeteaseMusic/server
- Reviewed commit: `39e21bfb4b7581f39785b190aeced201d23f0d41`
- License: GNU Lesser General Public License v3.0 only
- Used for: Kuwo mobile playback, `convert_url2`, DES protocol, and bounded
  fallback behavior research. Historical search endpoints are not treated as
  a current protocol baseline.

TuneWeave does not link or redistribute this LGPL-3.0-only implementation.
The Rust provider independently implements only protocol behavior verified
against current official Kuwo services.

## CharlesPikachu/musicdl

- Source: https://github.com/CharlesPikachu/musicdl
- Reviewed commit: `e623653d1db0cd8f6eadb7326cea57e2b2e3d6ad`
- License: PolyForm Noncommercial License 1.0.0
- Used for: candidate official Kuwo and Soda Music endpoints, media fields,
  quality tiers, share-page parsing, and lyric-flow research.

Only official-service protocol leads are considered. TuneWeave does not copy,
translate, bundle, or redistribute musicdl, and never adopts the project's
third-party hosted parser fallbacks.

## listen1/listen1-api

- Source: https://github.com/listen1/listen1-api
- Reviewed commit: `aa4b9d34aad577a254a70b2754415adcbb17294d`
- License: MIT
- Used for: historical Kuwo capability inventory, media URL categories, and
  Listen1 data-model research. The snapshot is not evidence that an endpoint
  remains usable.

## SaKongA/PopDownloader

- Source: https://github.com/SaKongA/PopDownloader
- Reviewed commit: `8e48fd1d01b7d3d4262149863818ae15ee7e3bc9`
- License: ISC is declared in `package.json`; the reviewed snapshot does not
  contain a root license text.
- Used for: Soda Music login transactions, account data, created and saved
  playlists, entitlement fields, media download, and local decryption research.

The snapshot is research-only. TuneWeave does not copy, translate, bundle, or
redistribute its source, and independently verifies official platform behavior.

## 520Qiuyu/qishuiMusicAnalysis

- Source: https://github.com/520Qiuyu/qishuiMusicAnalysis
- Reviewed commit: `b8f4e4f00be7c77ae6d12ca94d849c7f534cd3a9`
- License: no license file or license declaration was present in the reviewed
  snapshot.
- Used for: manual research into the Soda Music PC `track_v2` request shape,
  device identity, Cookie, `x-helios`, `x-medusa`, and response fields.

Because no license is granted, TuneWeave does not copy, translate, modify,
bundle, redistribute, or otherwise reuse this project's source. It is only an
index of protocol questions to validate independently against official traffic.

## baizeyv/SodaDownloader

- Source: https://github.com/baizeyv/SodaDownloader
- Reviewed commit: `893b49c35b7e11ada029e78782092f2553904281`
- License: MIT
- Used for: Soda Music share links, `aid` and session behavior, media quality
  selection, download flow, and encrypted-media interoperability research.

## naiyQAQ/qishui-decrypt

- Source: https://github.com/naiyQAQ/qishui-decrypt
- Reviewed commit: `d360c20a697f9988c6b567c924af5b9784d18390`
- License: MIT
- Used for: Soda Music `spade_a` key derivation, MP4 `senc` parsing, AES-CTR
  behavior, and FLAC/AAC/MP4 container reconstruction research.

Any future Rust implementation must independently enforce MP4 box bounds,
input/output limits, authorization boundaries, and real-media validation. CENC
handling must not be used to bypass membership or purchase restrictions.

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
