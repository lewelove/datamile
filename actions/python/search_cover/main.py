#!/usr/bin/env python3
import sys
import json
import argparse
import urllib.parse
import subprocess
from pathlib import Path

def main():
    parser = argparse.ArgumentParser(description="Search high-resolution album covers.")
    parser.add_argument("--artist", type=str, help="Album artist name")
    parser.add_argument("--album", type=str, help="Album title")
    parser.add_argument("--path", type=Path, help="Optional album directory containing album.lock.json")

    args = parser.parse_args()

    artist = args.artist or ""
    album = args.album or ""

    if args.path and (not artist or not album):
        lock_file = args.path.resolve() / "album.lock.json"
        if lock_file.exists():
            with open(lock_file, "r", encoding="utf-8") as f:
                data = json.load(f)
                meta = data.get("album", {}).get("tags", {})
                artist = artist or meta.get("albumartist") or meta.get("artist") or ""
                album = album or meta.get("album") or ""

    if not artist and not album:
        print("\033[31mError: Must provide --artist and --album or a valid --path with album.lock.json\033[0m")
        sys.exit(1)

    artist_encoded = urllib.parse.quote(artist)
    album_encoded = urllib.parse.quote(album)

    url = f"https://covers.musichoarders.xyz/?theme=dark&sources=amazonmusic,applemusic,deezer,discogs,fanarttv,lastfm,musicbrainz,qobuz,soulseek&country=US&artist={artist_encoded}&album={album_encoded}"

    launcher = "open" if sys.platform == "darwin" else "xdg-open"
    subprocess.Popen(
        [launcher, url],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )

if __name__ == "__main__":
    main()
