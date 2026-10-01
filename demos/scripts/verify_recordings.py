"""Decode every demo and check that the GIF/MP4 pair matches its current tape."""

from dataclasses import dataclass
import json
from pathlib import Path
import subprocess


@dataclass(frozen=True)
class MediaInfo:
    width: int
    height: int
    frames: int
    duration: float


def probe(path: Path) -> MediaInfo:
    result = subprocess.run(
        [
            "ffprobe", "-v", "error", "-count_frames", "-select_streams", "v:0",
            "-show_entries", "stream=width,height,nb_read_frames:format=duration",
            "-of", "json", str(path),
        ],
        check=True, capture_output=True, text=True, timeout=60,
    )
    if result.stderr.strip():
        raise ValueError(f"{path}: decoder reported errors: {result.stderr.strip()}")
    metadata = json.loads(result.stdout)
    stream = metadata["streams"][0]
    info = MediaInfo(
        int(stream["width"]), int(stream["height"]),
        int(stream["nb_read_frames"]), float(metadata["format"]["duration"]),
    )
    if min(info.width, info.height, info.frames) <= 0 or info.duration <= 0:
        raise ValueError(f"{path}: empty or invalid video: {info}")
    return info


def main() -> None:
    tapes = sorted(Path("demos/tapes").glob("*.tape"))
    if not tapes:
        raise ValueError("No tapes found. Run from the repository root.")
    for tape in tapes:
        gif = Path("demos/out") / f"{tape.stem}.gif"
        mp4 = gif.with_suffix(".mp4")
        for media in [gif, mp4]:
            if media.stat().st_mtime < tape.stat().st_mtime:
                raise ValueError(f"{media}: stale recording, render {tape} again")
        gif_info, mp4_info = probe(gif), probe(mp4)
        if (
            (gif_info.width, gif_info.height) != (mp4_info.width, mp4_info.height)
            or abs(gif_info.duration - mp4_info.duration) > 0.1
        ):
            raise ValueError(f"{tape.stem}: GIF and MP4 dimensions/duration differ")
        print(f"{tape.stem}: {mp4_info.width}x{mp4_info.height}, {mp4_info.duration:.2f}s, OK")
    print(f"Verified {len(tapes)} GIF/MP4 pairs ({len(tapes) * 2} media files).")


if __name__ == "__main__":
    main()
