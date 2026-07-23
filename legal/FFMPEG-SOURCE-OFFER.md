# FFmpeg corresponding source and relinking information

The Windows `mua_wav` binary is normally linked statically with the custom LGPL FFmpeg build
described by this repository's `vcpkg/ffmpeg` overlay and vcpkg manifest.

The complete corresponding FFmpeg source can be reproduced with the pinned vcpkg baseline,
overlay port, triplet, and build instructions committed with the matching `mua` release. Keep
the release source archive and build metadata available for at least the period required by the
LGPL. Distributors are responsible for supplying any additional object files or relinking method
required for their particular distribution.

FFmpeg upstream source: <https://ffmpeg.org/download.html>

