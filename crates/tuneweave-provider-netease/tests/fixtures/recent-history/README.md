# Recent-history fixtures

These synthetic responses contain no real account data. They fix the documented
`data.list` record shape (resourceId, resourceType, playTime, os,
multiTerminalInfo, data) and the existing NetEase song/album/playlist DTOs.
They are not claimed to be fresh authenticated captures.

The independently checked request protocol is WeAPI with a limit-only payload:
`/api/play-record/{song,album,playlist}/list`. Source snapshot:
NeteaseCloudMusicApiEnhanced/api-enhanced commit
`a8c781fd64faab17fedfd46e0615a2609307f163`, modules
`record_recent_song.js`, `record_recent_album.js`, `record_recent_playlist.js`.
See `docs/recent-history.md` for the normalized contract and limitations.
