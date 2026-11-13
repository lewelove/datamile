#!/usr/bin/env python3
import os
import sys
import json
import re
import argparse
import urllib.request
import urllib.parse
from pathlib import Path
import lyricsgenius

def trigger_update(server_url, album_id):
    encoded_id = urllib.parse.quote(album_id, safe='')
    url = f"{server_url.rstrip('/')}/api/update-album/{encoded_id}"
    req = urllib.request.Request(url, method="POST")
    try:
        urllib.request.urlopen(req, timeout=2)
    except Exception:
        pass

def clean_genius_lyrics(lyrics, title):
    if not lyrics:
        return ""

    lines = lyrics.split("\n")
    if lines and "Contributors" in lines[0]:
        lines.pop(0)

    filtered_lines = []
    for line in lines:
        trimmed = line.strip()
        if trimmed.startswith("[") and trimmed.endswith("]"):
            filtered_lines.append("")
            continue
        filtered_lines.append(trimmed)

    cleaned = "\n".join(filtered_lines)

    cleaned = re.sub(r"\(\s*\n\s*", "(", cleaned)
    cleaned = re.sub(r"\s*\n\s*\)", ")", cleaned)
    cleaned = re.sub(r"\n{3,}", "\n\n", cleaned)

    cleaned = re.sub(r"[0-9]*Embed$", "", cleaned)
    cleaned = cleaned.strip()

    return cleaned

def sanitize_filename(name):
    return re.sub(r'[<>:"/\\|?*]', '_', name)

def get_album_lyrics(target_dir, access_token, mpd_file, server_url):
    lock_file = target_dir / "album.lock.json"
    if not lock_file.exists():
        print(f"\033[31mError: album.lock.json not found in {target_dir}\033[0m")
        return False

    with open(lock_file, "r", encoding="utf-8") as f:
        album_lock = json.load(f)

    album_obj = album_lock.get("album", {})
    album_tags = album_obj.get("tags", {})
    album_id = album_obj.get("id", "")
    album_artist = album_tags.get("albumartist")
    total_discs = int(album_obj.get("objects", {}).get("info", {}).get("total_discs", 1))
    tracks = album_obj.get("tracks", [])

    if not album_artist or not tracks:
        print("\033[31mError: Invalid metadata structure in lock data.\033[0m")
        return False

    genius = lyricsgenius.Genius(access_token)
    genius.verbose = False
    genius.remove_section_headers = False

    lyrics_dir = target_dir / "Lyrics"
    lyrics_dir.mkdir(exist_ok=True)

    print(f"\033[1;36mFetching lyrics for: {album_artist} - {album_tags.get('album')}\033[0m")

    playing_idx = None
    if mpd_file:
        mpd_normalized = mpd_file.lstrip("/")
        for i, track in enumerate(tracks):
            t_path = track.get("location", {}).get("path", "")
            if t_path:
                track_abs = str((target_dir / t_path).resolve())
                if track_abs.endswith(f"/{mpd_normalized}") or track_abs == mpd_file:
                    playing_idx = i
                    break

    def fetch_for_track(track):
        track_tags = track.get("tags", {})
        title = track_tags.get("title")
        track_num = str(track_tags.get("tracknumber", "0")).zfill(2)
        disc_num = str(track_tags.get("discnumber", "1"))

        if not title:
            return

        safe_title = sanitize_filename(title)

        if total_discs > 1:
            filename = f"{disc_num}.{track_num} - {safe_title}.txt"
        else:
            filename = f"{track_num} - {safe_title}.txt"

        dest_path = lyrics_dir / filename

        if dest_path.exists():
            print(f"  \033[90mSkipping: {title} (File exists)\033[0m")
            return

        try:
            song = genius.search_song(title, album_artist)
            if song:
                cleaned_text = clean_genius_lyrics(song.lyrics, title)
                with open(dest_path, "w", encoding="utf-8") as lf:
                    lf.write(cleaned_text)
                print(f"  \033[32m✔ Saved: {title}\033[0m")
            else:
                print(f"  \033[33mNot found: {title}\033[0m")
        except Exception as e:
            print(f"  \033[31mError fetching {title}: {e}\033[0m")

    if playing_idx is not None:
        fetch_for_track(tracks[playing_idx])
        if album_id and server_url:
            trigger_update(server_url, album_id)

    for i, track in enumerate(tracks):
        if i == playing_idx:
            continue
        fetch_for_track(track)

    return True

def main():
    parser = argparse.ArgumentParser(description="Fetch album track lyrics from Genius.")
    parser.add_argument("--path", required=True, type=Path, help="Target album directory")
    parser.add_argument("--token", type=str, help="Genius access token")
    parser.add_argument("--playing-file", type=str, help="Currently playing track relative path")
    parser.add_argument("--server-url", type=str, default="http://127.0.0.1:8000", help="Datamile server API URL")

    args = parser.parse_args()
    target_dir = args.path.resolve()

    if not target_dir.is_dir():
        print(f"\033[31mError: Path '{target_dir}' is not a directory.\033[0m")
        sys.exit(1)

    token = args.token or os.environ.get("GENIUS_ACCESS_TOKEN") or os.environ.get("GENIUS_API_KEY")
    if not token:
        print("\033[31mError: Genius Access Token is required via --token or GENIUS_ACCESS_TOKEN env var.\033[0m")
        sys.exit(1)

    success = get_album_lyrics(target_dir, token, args.playing_file, args.server_url)
    if not success:
        sys.exit(1)

if __name__ == "__main__":
    main()
