#!/usr/bin/env python3
import sys
import json
import re
import argparse
import urllib.request
import urllib.parse
from pathlib import Path

def trigger_update(server_url, album_id):
    encoded_id = urllib.parse.quote(album_id, safe='')
    url = f"{server_url.rstrip('/')}/api/update-album/{encoded_id}"
    req = urllib.request.Request(url, method="POST")
    try:
        urllib.request.urlopen(req, timeout=2)
    except Exception:
        pass

def sanitize_filename(name):
    return re.sub(r'[<>:"/\\|?*]', '_', name)

def process_album(target_dir, auto_apply, server_url):
    lock_file = target_dir / "album.lock.json"
    if not lock_file.exists():
        print(f"\033[31mError: album.lock.json not found in {target_dir}\033[0m")
        return False

    with open(lock_file, "r", encoding="utf-8") as f:
        album_lock = json.load(f)

    album_obj = album_lock.get("album", {})
    tracks_data = album_obj.get("tracks", [])
    if not tracks_data:
        return False

    album_id = album_obj.get("id", "")

    try:
        total_discs = int(album_obj.get("objects", {}).get("info", {}).get("total_discs", 1))
    except (ValueError, TypeError):
        total_discs = 1

    track_nums = []
    disc_nums = []
    for t in tracks_data:
        tags = t.get("tags", {})
        try:
            track_nums.append(int(tags.get("tracknumber", 0)))
        except (ValueError, TypeError):
            track_nums.append(0)
        try:
            disc_nums.append(int(tags.get("discnumber", 1)))
        except (ValueError, TypeError):
            disc_nums.append(1)

    max_track_num = max(track_nums) if track_nums else 0
    max_disc_num = max(disc_nums + [total_discs]) if disc_nums else 1

    track_pad = max(2, len(str(max_track_num)))
    disc_pad = max(1, len(str(max_disc_num)))

    rename_tasks = []

    for t in tracks_data:
        tags = t.get("tags", {})
        t_location = t.get("location", {})
        rel_path = t_location.get("path")
        if not rel_path:
            continue

        old_file_path = target_dir / rel_path
        if not old_file_path.exists():
            continue

        try:
            track_num = int(tags.get("tracknumber", 0))
        except (ValueError, TypeError):
            track_num = 0

        try:
            disc_num = int(tags.get("discnumber", 1))
        except (ValueError, TypeError):
            disc_num = 1

        title = str(tags.get("title", ""))
        safe_title = sanitize_filename(title)
        ext = old_file_path.suffix

        track_str = str(track_num).zfill(track_pad)

        if total_discs >= 2:
            disc_str = str(disc_num).zfill(disc_pad)
            new_filename = f"{disc_str}.{track_str} - {safe_title}{ext}"
        else:
            new_filename = f"{track_str} - {safe_title}{ext}"

        new_file_path = old_file_path.parent / new_filename

        if old_file_path != new_file_path:
            rename_tasks.append({
                "old_path": old_file_path,
                "new_path": new_file_path,
                "rel_path": rel_path,
                "old_name": old_file_path.name,
                "new_name": new_filename
            })

    if not rename_tasks:
        return True

    print(f"\n\033[1;36m{target_dir.name}\033[0m")
    for task in rename_tasks:
        print(f"\033[1m🎵 {task['rel_path']}\033[0m")
        print(f"   \033[34m~ {task['old_name']} -> {task['new_name']}\033[0m")

    if not auto_apply:
        try:
            sys.stdout.write("\n\033[1;35mApply changes? [y/N]: \033[0m")
            sys.stdout.flush()
            with open("/dev/tty", "r") as tty:
                ans = tty.readline().strip().lower()
            if ans not in ("y", "yes"):
                return True
        except Exception:
            return True

    temp_tasks = []
    for idx, task in enumerate(rename_tasks):
        old_p = task["old_path"]
        new_p = task["new_path"]
        temp_p = old_p.with_name(f"{old_p.name}.tmp_rename_{idx}")
        old_p.rename(temp_p)
        temp_tasks.append((temp_p, new_p))

    for temp_p, new_p in temp_tasks:
        temp_p.rename(new_p)

    print("\033[32m✔ Done.\033[0m")
    if album_id and server_url:
        trigger_update(server_url, album_id)
    return True

def main():
    parser = argparse.ArgumentParser(description="Standardize track file names from metadata.")
    parser.add_argument("--path", required=True, type=Path, help="Target album directory")
    parser.add_argument("-y", "--auto", action="store_true", help="Apply renames without confirmation")
    parser.add_argument("--server-url", type=str, default="http://127.0.0.1:8000", help="Datamile server API URL")

    args = parser.parse_args()
    target_dir = args.path.resolve()

    if not target_dir.is_dir():
        print(f"\033[31mError: Path '{target_dir}' is not a directory.\033[0m")
        sys.exit(1)

    success = process_album(target_dir, args.auto, args.server_url)
    if not success:
        sys.exit(1)

if __name__ == "__main__":
    main()
