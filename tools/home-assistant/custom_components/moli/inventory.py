"""Safe, typed projection of HA entity-registry rows into the Moli catalog."""

from __future__ import annotations

import hashlib
import json
import math
from typing import Any, Iterable

from .protocol import MAX_TEXT_BYTES, ProtocolError, valid_opaque

SUPPORTED_DOMAINS = frozenset({
    "binary_sensor", "sensor", "switch", "light", "cover", "climate", "number",
    "input_number", "select", "input_select", "input_boolean",
})
EXCLUDED_DOMAINS = frozenset({"alarm_control_panel", "person", "zone", "lock", "input_text"})
MAX_DEVICES = 1024
MAX_CHANNELS = 4096
MAX_CATALOGUE_BYTES = 1024 * 1024
SEMANTIC_CLASSES = frozenset({
    "temperature", "humidity", "battery", "illuminance", "power", "energy",
    "voltage", "current", "pressure",
})


def _stable_id(prefix: str, value: str) -> str:
    return f"{prefix}-{hashlib.sha256(value.encode()).hexdigest()[:32]}"


def _bounded_text(value: Any) -> str | None:
    if not isinstance(value, str) or len(value.encode()) > MAX_TEXT_BYTES:
        return None
    if any(ord(character) < 32 for character in value):
        return None
    return value


def _kind_and_value(
    domain: str, state: str, options: Any, *, unit: str | None = None,
    device_class: str | None = None,
) -> tuple[dict, Any] | None:
    if domain in {"binary_sensor", "switch", "light", "input_boolean"}:
        if state == "on":
            return ({"type": "binary"}, True)
        if state == "off":
            return ({"type": "binary"}, False)
        return ({"type": "binary"}, None)

    if domain in {"select", "input_select", "climate"} or (
        domain == "sensor" and device_class == "enum"
    ):
        if not isinstance(options, list) or not 1 <= len(options) <= 256:
            return None
        native_values = sorted(set(option for option in options if _bounded_text(option) is not None))
        if len(native_values) != len(options):
            return None
        codes = {native: code for code, native in enumerate(native_values)}
        choices = [
            {"code": code, "native": native, "label": native}
            for native, code in codes.items()
        ]
        catalogue = int(hashlib.sha256("\0".join(native_values).encode()).hexdigest()[:8], 16) or 1
        value = codes.get(state)
        return ({"type": "enum_code", "catalogue": catalogue, "version": 1, "choices": choices}, value)

    if domain == "cover":
        fixed_options = ["closed", "closing", "open", "opening"]
        codes = {native: code for code, native in enumerate(fixed_options)}
        choices = [
            {"code": code, "native": native, "label": native}
            for native, code in codes.items()
        ]
        return (
            {"type": "enum_code", "catalogue": 1, "version": 1, "choices": choices},
            codes.get(state),
        )

    if domain in {"number", "input_number"}:
        try:
            number = float(state)
        except (TypeError, ValueError):
            number = math.nan
        return ({"type": "numeric"}, number if math.isfinite(number) else None)

    try:
        number = float(state)
    except (TypeError, ValueError):
        number = math.nan
    if math.isfinite(number):
        return ({"type": "numeric"}, number)
    if state in {"unknown", "unavailable", ""}:
        if unit or device_class in {
            "apparent_power", "aqi", "carbon_dioxide", "carbon_monoxide", "current",
            "data_rate", "data_size", "distance", "duration", "energy", "enum",
            "frequency", "gas", "humidity", "illuminance", "irradiance", "moisture",
            "monetary", "nitrogen_dioxide", "nitrogen_monoxide", "nitrous_oxide",
            "ozone", "ph", "pm1", "pm10", "pm25", "power", "precipitation",
            "pressure", "reactive_power", "signal_strength", "sound_pressure",
            "speed", "sulphur_dioxide", "temperature", "volatile_organic_compounds",
            "voltage", "volume", "volume_flow_rate", "water", "weight",
        }:
            return ({"type": "numeric"}, None)
        return ({"type": "text"}, None)
    if domain != "sensor" or device_class not in {"date", "timestamp"}:
        return None
    value = _bounded_text(state)
    if value is None:
        return None
    return ({"type": "text"}, value)


def build_inventory(
    rows: Iterable[dict[str, Any]], *, installation: str, revision: int
) -> tuple[dict[str, Any], dict[tuple[str, str], dict[str, Any]]]:
    """Build a catalog and matching value messages from prefiltered safe rows.

    Rows must contain entity_id, registry_entity_id, device_id, device_name,
    name, domain, state, unit, and optional enum_options. Callers pass only
    enabled registry entities and never pass the entity attributes wholesale.
    """
    if not valid_opaque(installation) or not isinstance(revision, int) or isinstance(revision, bool) or revision < 1:
        raise ProtocolError("catalogue_revision")
    groups: dict[str, dict[str, Any]] = {}
    states: dict[tuple[str, str], dict[str, Any]] = {}
    accepted = 0
    estimated_bytes = 256
    for row in rows:
        domain = row.get("domain")
        if not isinstance(domain, str) or domain not in SUPPORTED_DOMAINS or domain in EXCLUDED_DOMAINS:
            continue
        entity_id = row.get("entity_id")
        registry_entity_id = row.get("registry_entity_id")
        state_value = row.get("state")
        if not isinstance(entity_id, str) or not isinstance(registry_entity_id, str):
            continue
        if not valid_opaque(entity_id) or not valid_opaque(registry_entity_id):
            continue
        state_text = state_value if isinstance(state_value, str) else "unavailable"
        if domain == "cover" and row.get("cover_feedback") is not True:
            state_text = "unknown"
        typed = _kind_and_value(
            domain, state_text, row.get("enum_options"),
            unit=row.get("unit"), device_class=row.get("device_class"),
        )
        if typed is None:
            continue
        kind, value = typed
        device_source = row.get("device_id") or registry_entity_id
        device_id = _stable_id("device", str(device_source))
        channel_id = _stable_id("channel", registry_entity_id)
        if device_id not in groups and len(groups) >= MAX_DEVICES:
            continue
        if accepted >= MAX_CHANNELS:
            break
        device_name = _bounded_text(row.get("device_name")) or "Home Assistant device"
        name = _bounded_text(row.get("name")) or entity_id
        unit = _bounded_text(row.get("unit"))
        actions = row.get("actions", [])
        if domain != "cover" or not isinstance(actions, list):
            actions = []
        actions = [action for action in actions if action in {"open", "close", "stop"}]
        channel = {
            "id": channel_id,
            "entity_id": entity_id,
            "registry_entity_id": registry_entity_id,
            "name": name,
            "unit": unit,
            "kind": kind,
            "semantic": row.get("device_class") if row.get("device_class") in SEMANTIC_CLASSES else None,
            "readable": True,
            "actions": actions,
        }
        estimated_channel_bytes = (
            256 + len(entity_id.encode()) + len(registry_entity_id.encode())
            + len(name.encode()) + (len(unit.encode()) if unit else 0)
            + len(json.dumps(kind, ensure_ascii=False, separators=(",", ":")).encode())
        )
        new_device_bytes = (
            128 + len(device_name.encode()) if device_id not in groups else 0
        )
        if estimated_bytes + estimated_channel_bytes + new_device_bytes > MAX_CATALOGUE_BYTES:
            continue
        device = groups.setdefault(device_id, {
            "id": device_id,
            "name": device_name,
            "channels": [],
        })
        device["channels"].append(channel)
        estimated_bytes += estimated_channel_bytes + new_device_bytes
        states[(device_id, channel_id)] = {
            "v": 1,
            "installation": installation,
            "catalogue_revision": revision,
            "device": device_id,
            "channel": channel_id,
            "value": value,
            "available": value is not None,
        }
        accepted += 1
    catalog = {
        "v": 1,
        "installation": installation,
        "revision": revision,
        "devices": list(groups.values()),
    }
    if len(json.dumps(catalog, ensure_ascii=False, separators=(",", ":")).encode()) > MAX_CATALOGUE_BYTES:
        raise ProtocolError("catalogue_bound")
    return catalog, states


def value_for_declared_kind(kind: dict[str, Any], state: str) -> Any:
    """Convert a live HA state without changing its catalog-declared type."""
    kind_type = kind.get("type")
    if kind_type == "binary":
        return True if state == "on" else False if state == "off" else None
    if kind_type == "numeric":
        try:
            number = float(state)
        except (TypeError, ValueError):
            return None
        return number if math.isfinite(number) else None
    if kind_type == "text":
        return None if state in {"", "unknown", "unavailable"} else _bounded_text(state)
    if kind_type == "enum_code":
        for choice in kind.get("choices", []):
            if choice.get("native") == state:
                return choice.get("code")
    return None
