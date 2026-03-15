#!/usr/bin/env python3
"""Migrate legacy single-file runtime config into split config + transports files."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

RUNTIME_KEYS = {
    "protocol_version",
    "shutdown_timeout_ms",
    "ready_timeout_ms",
    "require_all_ready",
    "restart_intensity_max_restarts",
    "restart_intensity_window_seconds",
    "backoff_min_ms",
    "backoff_max_ms",
    "router_delivery_timeout_ms",
    "router_degraded_drop_threshold",
}

CATALOG_KEYS = {
    "name",
    "transport",
    "enabled",
    "instance_id",
    "subscriptions",
    "buses",
    "bus",
    "config_path",
    "config",
    "type",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Convert a legacy mixed config JSON into split runtime config and "
            "transport catalog files."
        )
    )
    parser.add_argument("input", help="Path to legacy mixed JSON config")
    parser.add_argument(
        "--runtime-out",
        default="config.json",
        help="Output path for runtime config JSON (default: %(default)s)",
    )
    parser.add_argument(
        "--transports-out",
        default="transports.json",
        help="Output path for transports catalog JSON (default: %(default)s)",
    )
    parser.add_argument(
        "--transport-config-dir",
        default=".",
        help=(
            "Directory used for generated per-transport config files when inline "
            "config is present (default: current directory)"
        ),
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite existing output files and generated transport config files",
    )
    return parser.parse_args()


def fail(message: str) -> None:
    raise SystemExit(message)


def ensure_writable(path: Path, force: bool) -> None:
    if path.exists() and not force:
        fail(f"Refusing to overwrite existing file: {path} (use --force)")
    path.parent.mkdir(parents=True, exist_ok=True)


def build_generated_filename(name: str, instance_id: str, index: int) -> str:
    safe_name = "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in name)
    safe_instance = "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in instance_id)
    return f"transport.{safe_name}.{safe_instance}.{index}.json"


def transport_catalog_entry(
    transport: dict[str, Any],
    index: int,
    config_dir: Path,
    force: bool,
) -> dict[str, Any]:
    name = transport.get("name") or transport.get("transport")
    if not isinstance(name, str) or not name:
        fail(f"transport entry #{index} missing non-empty 'name'/'transport'")

    instance_id = transport.get("instance_id")
    if not isinstance(instance_id, str) or not instance_id:
        instance_id = f"{name}_{index}"

    enabled = transport.get("enabled", True)
    if not isinstance(enabled, bool):
        fail(f"transport entry #{index} has non-boolean 'enabled'")

    subscriptions = transport.get("subscriptions")
    if subscriptions is None:
        if isinstance(transport.get("buses"), list):
            subscriptions = transport["buses"]
        elif isinstance(transport.get("bus"), str):
            subscriptions = [transport["bus"]]
        else:
            subscriptions = []
    if not isinstance(subscriptions, list) or any(not isinstance(x, str) for x in subscriptions):
        fail(f"transport entry #{index} has invalid subscriptions/buses")

    config_path = transport.get("config_path")
    if isinstance(config_path, str) and config_path:
        resolved_config_path = config_path
    else:
        inline_config = transport.get("config")
        if inline_config is None:
            inline_config = {
                key: value
                for key, value in transport.items()
                if key not in CATALOG_KEYS and key not in RUNTIME_KEYS
            }

        if not isinstance(inline_config, dict) or len(inline_config) == 0:
            fail(
                f"transport entry #{index} has no config_path and no inline transport config to extract"
            )

        generated_name = build_generated_filename(name, instance_id, index)
        generated_path = config_dir / generated_name
        ensure_writable(generated_path, force)
        generated_path.write_text(json.dumps(inline_config, indent=2) + "\n", encoding="utf-8")
        resolved_config_path = generated_name

    return {
        "name": name,
        "enabled": enabled,
        "instance_id": instance_id,
        "subscriptions": subscriptions,
        "config_path": resolved_config_path,
    }


def main() -> None:
    args = parse_args()

    input_path = Path(args.input)
    runtime_out = Path(args.runtime_out)
    transports_out = Path(args.transports_out)
    transport_config_dir = Path(args.transport_config_dir)

    if not input_path.is_file():
        fail(f"Input file not found: {input_path}")

    raw = input_path.read_text(encoding="utf-8")
    data = json.loads(raw)

    if not isinstance(data, dict):
        fail("Top-level JSON must be an object")

    transports = data.get("transports")
    if not isinstance(transports, list):
        fail("Expected top-level 'transports' array in input config")

    runtime = {key: data[key] for key in RUNTIME_KEYS if key in data}
    catalog = {
        "transports": [
            transport_catalog_entry(item, idx, transport_config_dir, args.force)
            for idx, item in enumerate(transports)
        ]
    }

    ensure_writable(runtime_out, args.force)
    ensure_writable(transports_out, args.force)

    runtime_out.write_text(json.dumps(runtime, indent=2) + "\n", encoding="utf-8")
    transports_out.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")

    print(f"Wrote runtime config: {runtime_out}")
    print(f"Wrote transports catalog: {transports_out}")
    print(f"Generated transport configs directory: {transport_config_dir}")


if __name__ == "__main__":
    main()
