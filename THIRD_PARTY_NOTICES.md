# Third-party notices

TuneWeave is an independent Rust implementation informed by the public protocol
research and implementations listed below. Source snapshots are recorded so
future ports can be audited precisely.

## NeteaseCloudMusicApiEnhanced/api-enhanced

- Source: https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced
- Reviewed commit: `6946dc8e14b6fb125191bc43525d4faa8123d8ae`
- License: MIT
- Used for: NetEase Cloud Music request protocols, endpoint behavior, response
  normalization, and authentication flow research.

## MOPELotus/Lotus-ReFactor

- Source: https://github.com/MOPELotus/Lotus-ReFactor
- Reviewed commit: `646400c1cf098c3887ef90886617625169fb58de`
- License: Lotus-ReFactor Source-Available Proprietary License
- Used for: NetEase Music Partner request logic and implementation details.
- Authorization: MOPELotus states that they contributed 100% of
  Lotus-ReFactor and explicitly authorizes TuneWeave to reference and reuse its
  logic and implementation. The public Lotus-ReFactor license is still recorded
  here accurately for third-party readers.

## L-1124/QQMusicApi

- Source: https://github.com/L-1124/QQMusicApi
- Reviewed commit: `b859d8e01566b92c27e78dd400f4f8c6950685f2`
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
- Reviewed commit: `45cda48aeee995121ff7987a81e52949732a917c`
- License: Apache License 2.0
- Used for: Migu catalog, login, PACM token, resource identity, entitlement,
  and media URL behavior research.

## qyhqiu/kuwoMusicApi

- Source: https://github.com/qyhqiu/kuwoMusicApi
- Reviewed commit: `e8e720b90b4d7e3052078a3380906f2b3349e388`
- Repository metadata: the root `LICENSE` contains Apache-2.0 while
  `package.json` declares ISC. TuneWeave will not port this implementation
  until the relevant endpoints and applicable license are revalidated.
- Used for: preliminary Kuwo endpoint inventory only.

## MOPELotus/BBDown

- Source: https://github.com/MOPELotus/BBDown
- Reviewed commit: `259a5558cee0a349a7ebb60bd31e40c88e5bc1ed`
- License: MIT
- Used for: Bilibili identifier parsing, metadata, multipart video, DASH audio
  and video track, authentication, and request header behavior research.

## TuneWeave license texts

TuneWeave is available under either MIT or Apache-2.0 at the user's option.
The choice is summarized in `LICENSE`; complete texts are in `LICENSE-MIT` and
`LICENSE-APACHE`. Required third-party notices are retained under `licenses/`.
