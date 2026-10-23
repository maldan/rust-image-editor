# Native video previews

Video uses `ffmpeg-next` and the FFmpeg shared libraries in the editor process.
It does not launch `ffmpeg.exe`, serialize frames through a pipe, or extract MP4s
to temporary files. Generated videos are read from their shared memory buffer;
saved videos are read in chunks directly from their immutable `.mgimg` snapshot.

## Windows x64 development

Run once from PowerShell:

```powershell
./scripts/setup-video.ps1
cargo build
cargo test
```

The setup script downloads checksum-pinned dependencies into ignored `.deps/`:

- FFmpeg 9.0 shared LGPL build from BtbN, revision `4cf298e66a`, release
  `autobuild-2026-10-22-13-06`:
  <https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-10-22-13-06>.
- libclang 18.1.1 from the PyPI `libclang` wheel, used only to generate bindings
  during compilation: <https://pypi.org/project/libclang/18.1.1/>.

No executable from the FFmpeg archive is extracted. The generated local
`.cargo/config.toml` sets `FFMPEG_DIR` and `LIBCLANG_PATH`; an existing different
config is preserved. An existing compatible SDK can instead be supplied through
those environment variables. The setup script can be rerun to restore dependencies.

`build.rs` stages the required DLLs next to the application and test executables.
For a portable copy, keep these files together:

```text
image-editor.exe
avcodec-63.dll
avformat-63.dll
avutil-61.dll
swscale-10.dll
swresample-7.dll
FFMPEG-LICENSE.txt
```

The decoder neither needs a system FFmpeg installation nor searches PATH for a
video executable. libclang and the SDK headers/import libraries are build tools;
they are not required to run the editor. For other platforms, provide the FFmpeg
development libraries through the discovery mechanisms supported by
`ffmpeg-sys-next` (normally pkg-config).

## Playback and compatibility

- Existing videos, thumbnails and project format remain compatible.
- Decode and color conversion run on a background worker, with two codec threads.
- Each active preview has a 2 GiB LRU cache of presented RGBA frames, populated
  during playback and seeking. Memory is allocated on demand, not reserved upfront.
  The canvas and cache share pixel buffers; cache hits do not copy pixels. Decoder
  buffers and the current/pending presentation frames are additional to this limit.
  Closing or switching the video releases its cache. It does not predecode the
  entire video.
- Cached seeks show the retained frame immediately. On pause they skip decoding;
  during playback the decoder catches up in the background to continue from there.
  Uncached seeks decode from a keyframe but skip RGBA conversion and copying for
  all preroll frames. New seek requests cancel obsolete preroll between frames.
- The worker reads at most one presentation frame ahead and follows source frame
  timestamps. There is no fixed 25 FPS conversion.
- Preview size remains capped at 1920×1080, preserving aspect ratio. Posters use
  the same first-frame thumbnail logic and archive paths as before.
- Cancellation interrupts stream reads and presentation waits; the UI does not
  wait for the worker to finish. Looping seeks and flushes the decoder after its
  delayed frames have been drained.

This change uses software decoding and RGBA uploads to the existing canvas.
Audio playback and hardware decoding are separate future changes. To roll back
the decoder, revert the native-video code/dependency changes; saved projects and
video assets need no conversion.

## Asset properties and transport

The right dock tab is `Layers` only for documents. Image and video previews show
`Properties` instead, keeping the dock layout and restoring Layers on return.
Images show name, resolution, file size, decoded pixel size and zoom. Video shows
source dimensions, file size, duration, timeline position and playback controls.

Play/pause and seeking are transient preview settings. Seeking on pause displays
that frame without resuming playback, and newer scrub requests supersede older
ones. FPS defaults to the original stream rate; lowering it skips frames while
preserving source timing and duration. Values above the source rate have no effect.
The reset icon restores the original FPS. The MP4 is not modified.
Paused playback waits on a condition variable and does not keep decoding or
requesting canvas redraws. Source timestamps still govern default playback.
