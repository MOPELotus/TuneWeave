These fixtures contain newly generated one-second sine waves, with no service
media, credentials, or downloaded songs. FFmpeg independently encoded two
16-bit and five 24-bit FLAC files. Tests do not invoke FFmpeg or contact a music
service.

Generation commands (FFmpeg 6.1.1-3ubuntu5+esm13):

```sh
ffmpeg -v error -f lavfi -i 'aevalsrc=0.125*sin(2*PI*440*t)|0.0625*sin(2*PI*880*t):s=44100:d=1' -sample_fmt s16 -c:a flac -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-stereo.flac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.0625*sin(2*PI*330*t):s=48000:d=1' -sample_fmt s16 -c:a flac -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-mono.flac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.125*sin(2*PI*440*t)|0.0625*sin(2*PI*880*t):s=44100:d=1' -sample_fmt s32 -c:a flac -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-stereo-24.flac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.125*sin(2*PI*440*t)|0.0625*sin(2*PI*880*t):s=96000:d=1' -sample_fmt s32 -c:a flac -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-stereo-24-96k.flac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.125*sin(2*PI*440*t)|0.0625*sin(2*PI*880*t):s=88200:d=1' -sample_fmt s32 -c:a flac -frame_size 8192 -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-stereo-24-88k2.flac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.125*sin(2*PI*440*t)|0.0625*sin(2*PI*880*t):s=176400:d=1' -sample_fmt s32 -c:a flac -frame_size 8192 -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-stereo-24-176k4.flac
ffmpeg -v error -f lavfi -i 'aevalsrc=0.125*sin(2*PI*440*t)|0.0625*sin(2*PI*880*t):s=192000:d=1' -sample_fmt s32 -c:a flac -frame_size 8192 -map_metadata -1 -fflags +bitexact -flags:a +bitexact -y synthetic-stereo-24-192k.flac
ffmpeg -v error -i synthetic-stereo.flac -f md5 -
ffmpeg -v error -i synthetic-mono.flac -f md5 -
ffmpeg -v error -i synthetic-stereo-24.flac -c:a pcm_s24le -f s24le - | md5sum
ffmpeg -v error -i synthetic-stereo-24-96k.flac -c:a pcm_s24le -f s24le - | md5sum
ffmpeg -v error -i synthetic-stereo-24-88k2.flac -c:a pcm_s24le -f s24le - | md5sum
ffmpeg -v error -i synthetic-stereo-24-176k4.flac -c:a pcm_s24le -f s24le - | md5sum
ffmpeg -v error -i synthetic-stereo-24-192k.flac -c:a pcm_s24le -f s24le - | md5sum
```

The independently decoded interleaved signed 16-bit little-endian PCM MD5 values
are `bd4abb9f846beb89774eeb2d2d8a1707` (stereo, 44,100 samples) and
`5f3aaad670cd52f6a792ab5f91b22f6f` (mono, 48,000 samples). Both files use 4,608
samples per full frame and have a shorter final frame. All seven fixtures use
FLAC fixed-block strategy (the first audio-frame blocking-strategy bit is zero).
The 44.1 kHz stereo fixtures contain nine 4,608-sample frames followed by a
2,628-sample frame; the 48 kHz mono fixture contains ten full frames followed by
1,920 samples.
The 24-bit stereo/44.1 kHz PCM MD5 is
`427c9955d855b94f82ccd651fff04373`; its full frames contain 4,608 samples and
its final frame is shorter. FFprobe 6.1.1-3ubuntu5+esm13 independently reports
44,100 Hz, 2 channels and 24 raw bits per sample. The fixture SHA-256 is
`6e897d5f10db5ad944e8f3638e2f8a840ea1b97a9bfe036c60e2c4b797f61b0c`.
The 24-bit stereo/96 kHz fixture contains eleven 8,192-sample frames and a
5,888-sample tail. Its PCM MD5 is `4ab37a36b35fbfdadf32177a0b97e67f` and file
SHA-256 is `d61fd2df5986c9b4afedb9ca93ff2f199d953b5dbb5191f130f3fb2980376ec3`.
FFprobe independently reports 96,000 Hz, 2 channels and 24 raw bits per sample.

The three additional stereo/24-bit fixtures explicitly use 8,192 samples per
full frame to remain within the existing decoder allocation limit. Independent
FFmpeg decoding reports these interleaved signed 24-bit little-endian PCM MD5
values; FFprobe confirms the rate, bit depth, channels, frame sizes and exact
one-second sample count:

| Fixture rate | Full frames | Final frame samples | PCM MD5 | File SHA-256 |
| --- | ---: | ---: | --- | --- |
| 88.2 kHz | 10 | 6,280 | `decf6d806143788ba445674b3b8cb872` | `0c55f7f0b8b897c98a1b4ab1a300438231248fe8f7c92caf50d0beefd63c90bf` |
| 176.4 kHz | 21 | 4,368 | `bdc173a1c6137630519f444b37edd859` | `81ce2a8a3b33e7858da8a85e022b414ecadbcd83d6c87a32bbe9009e58ec4437` |
| 192 kHz | 23 | 3,584 | `48c930ee24dfe31c7a1ec1fd64b8172a` | `ef82d6254992ffee371b4404d04219bec839176a128ebee66faa6b9efe52e510` |

The fixed-block regression asserts Claxon 0.4.3 reports `Block.time()` as the
current frame block size multiplied by the encoded frame number, and that this
recovers the sequential frame number even for the short tail. Separately, the
cumulative decoded sample offset before every fixed frame must equal the
previous full-block size times that frame number, and the total must match
STREAMINFO. Thus the final stereo frame starts at sample 41,472 (9 × 4,608),
although Claxon reports its `Block.time()` as 23,652 (9 × 2,628). The parser
continues to verify the Claxon frame-time/frame-number relation, full blocks
before the tail, and exact accumulated total; no timestamp algorithm change is
needed for these fixtures.

The supported provider subset requires the existing ordinary SQ or ZQ24 grant,
full content, the supported encryption policy, a 32-character hexadecimal
`fileKey` for MG3D, and selected-account/native-UID and HTTPS CDN checks.
FLAC validation requires
16-bit mono/stereo SQ at 44.1/48 kHz and 24-bit mono/stereo ZQ24 at
44.1/48/88.2/96/176.4/192 kHz, explicit frame
bit depth, inherited or standard frame rate codes, a nonzero PCM MD5 and sample
count, at most one hour of audio, 64 MiB content, 1 MiB metadata, 128 metadata
blocks and an 8,192-sample maximum declared block. ZQ24 requires a fresh resource
format association and a matching FLAC grant; formatId alone does not prove
quality or rights. Other bit depths and sample rates fail closed. Unknown or
incomplete encodings fail without falling back to playback or exposing partial
plaintext. MGM and cloud content remain outside this slice. These synthetic
tests do not constitute real-account or platform-media acceptance.

First-party format and consumer trace is from official MobileMusic 8.9.1 APK
SHA-256 `74d2f2084601534457a4ccc843f6cd7983586ac3e92e0083bdc47ace7a9fa16c`.
The recovered base runtime DEX is
`d3eff909aea4b7127ed1f0cb2c30600552206b31e923da78e174c99538bab8bd`. Direct
decompilation inputs: `SongConsts.java` has SHA-256
`f7346a199de47408d63b5419f8c96406b19015d1c661b16a00fa1fa107bd3265`, and
`Song.java` has SHA-256
`5036a1ef1be3929a297c3a4ddd74e2cb9abda003dc3194b540e348375429479b` (JADX
1.5.6, single-class extraction). Extracted first-party consumer SHA-256 values:
`DownloadStrategyUtils.java` `7446cdc8258cbe1c666fdf7bc97758825b503f428ba649ca67e472a1cf7f1b4e`,
`DownloadRequestUtils.java` `f17259aaed9230c48999b9238af08188fccb0f064d6a486ca9e2098e7eee5155`,
`DownloadSongItem.java` `8157be25df4b4d905db668c58a05236a05622c12d1bc7bfbf37da83725c84f62`,
`DownloadTaskRunnable.java` `d9f38d80aebf4bbc6dc34933501355e80ab431fd737fc1550a54d6846bdfc265`,
and `ListenUrlUtils.java`
`b807b26e67cf3ff0556c5f0759e8654a918caa096739d06d645cfe0736acf03e`.
The trace is: `SongConsts.BIT24_FORMAT` is `011005` and
`PLAY_LEVEL_BIT24_HIGH` is `ZQ24`; `Song.initFormatInfo` puts that tone in the
dedicated bit-24 `SongFormat`; `DownloadStrategyUtils.getAllToneQuality` includes
it and carries its format code; `DownloadRequestUtils.simpleRequestDownloadUrl`
sends the selected tone as `formatType`; `DownloadSongItem` exposes
`fileKey`/`formatId`/`suffix`/`encryptionType`; and `DownloadTaskRunnable` uses
any nonempty `fileKey` to choose the MG3D container. `ListenUrlUtils` separately
requires the bit-24 format before selecting ZQ24. Rust still requires the fresh
resource format list to associate `formatId` with ZQ24, then validates the full
grant and actual 24-bit FLAC bytes; formatId by itself is not treated as rights.

The same official APK's `FFPlayer` uses `CMCCMediaPlayer`, whose native library
links both `libmg_corner.so` and `libavcodec_tsg.so`. The latter's SHA-256 is
`523b52e1ab15ad7eec19e8d3d5098c55b02a5de6c61cb50bc134aaa9a54bff71`.
Its FLAC frame-header consumer at ARM64 address `0x163778` reads the four-bit
sample-rate code and resolves code 11 to 96,000 Hz via table `0x8a1d8` at
`0x16399c` (GOT entry `0x54c850`). This establishes format consumption, not
rights to a particular song; the ordinary ZQ24 grant, actual 24-bit stream and
selected account must still be independently verified. The same table and
consumer resolve code 1 to 88,200 Hz, code 2 to 176,400 Hz, and code 3 to
192,000 Hz. These standard explicit frame-rate codes must match STREAMINFO;
extended rate encodings remain outside this subset. The additional rates
require the same ZQ24 authorization and actual 24-bit content. They do not
enable SQ/16-bit at any of these high sample rates, or change the 8,192-sample
block limit. Decoder support alone does not grant access to a resource.
