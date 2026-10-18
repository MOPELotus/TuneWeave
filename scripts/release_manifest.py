#!/usr/bin/env python3
"""Generate and check the compatibility declaration in the release manifest."""
import argparse
import json
from pathlib import Path
import re

# Change only when the documented client contract changes, not for each release.
COMPATIBILITY_PROFILES = ("tuneweave-http-v1",)
MAX_MANIFEST_BYTES = 128 * 1024
MAX_PROFILES = 16
PROFILE = re.compile(r"[a-z0-9][a-z0-9._-]{0,63}", re.ASCII)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate manifest field: {key}")
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError(f"Non-finite JSON constant: {value}")


def load_manifest(path):
    with Path(path).open("rb") as stream:
        raw = stream.read(MAX_MANIFEST_BYTES + 1)
    if len(raw) > MAX_MANIFEST_BYTES:
        raise ValueError("Release manifest exceeds size limit")
    value = json.loads(raw, object_pairs_hook=unique_object, parse_constant=reject_constant)
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value["schema_version"] != 1:
        raise ValueError("Unsupported release manifest schema")
    return value


def validate_profiles(value):
    profiles = value.get("compatibility_profiles")
    if not isinstance(profiles, list) or not 1 <= len(profiles) <= MAX_PROFILES:
        raise ValueError("compatibility_profiles must contain 1 to 16 profiles")
    if any(not isinstance(profile, str) or PROFILE.fullmatch(profile) is None for profile in profiles):
        raise ValueError("Invalid compatibility profile identifier")
    if len(set(profiles)) != len(profiles):
        raise ValueError("Duplicate compatibility profile")
    # Unknown identifiers are valid declarations for future clients. Consumers
    # must not infer compatibility when none of their supported profiles match.
    return profiles


def generated_manifest(value):
    result = dict(value)
    result["compatibility_profiles"] = list(COMPATIBILITY_PROFILES)
    validate_profiles(result)
    return result


def check_manifest(value):
    validate_profiles(value)
    if value != generated_manifest(value):
        raise ValueError("Manifest profiles differ from the source contract; regenerate with --write")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).resolve().parents[1] / "release-manifest.json")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        value = load_manifest(args.manifest)
        if args.check:
            check_manifest(value)
        else:
            target = args.manifest if args.write else args.output
            target.write_text(json.dumps(generated_manifest(value), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except (ValueError, OSError) as error:
        parser.exit(1, f"Release manifest validation failed: {error}\n")


if __name__ == "__main__":
    main()
