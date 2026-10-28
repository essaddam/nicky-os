"""Fail-closed migration request verification for the HA companion."""

from __future__ import annotations

import hashlib
import json
import re
from collections.abc import Callable
from typing import Any

from .protocol import (
    MAX_U64,
    ProtocolError,
    QuiescenceObservation,
    proof_observation_complete,
    valid_opaque,
)


def _canonical_hash(value: Any) -> str:
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def parse_migration_request(
    payload: bytes | str, *, installation: str, session: str, now_ms: int
) -> dict[str, Any]:
    from .protocol import decode_json

    request = decode_json(payload, limit=1024 * 1024)
    if set(request) != {"v", "installation", "session", "expires_at_ms", "binding"}:
        raise ProtocolError("migration_shape")
    if (
        not isinstance(request["v"], int)
        or isinstance(request["v"], bool)
        or request["v"] != 1
        or request["installation"] != installation
        or request["session"] != session
    ):
        raise ProtocolError("migration_identity")
    expires = request["expires_at_ms"]
    if not isinstance(expires, int) or isinstance(expires, bool) or not now_ms < expires <= now_ms + 5000:
        raise ProtocolError("migration_expired")
    binding = request["binding"]
    expected_binding_fields = {
        "key", "native_device", "phase", "revision", "generation", "operation_id",
        "plan_hash", "plan", "reconcile_required",
    }
    if not isinstance(binding, dict) or set(binding) != expected_binding_fields:
        raise ProtocolError("migration_binding")
    key = binding["key"]
    plan = binding["plan"]
    if not isinstance(key, dict) or not isinstance(plan, dict):
        raise ProtocolError("migration_binding")
    if (
        key != plan.get("binding")
        or binding["native_device"] != plan.get("native_device")
        or binding["plan_hash"] != _canonical_hash(plan)
        or plan.get("version") != 1
        or not isinstance(binding["revision"], int)
        or isinstance(binding["revision"], bool)
        or binding["revision"] < 1
        or binding["revision"] > MAX_U64
        or not isinstance(binding["generation"], int)
        or isinstance(binding["generation"], bool)
        or binding["generation"] < 1
        or binding["generation"] > MAX_U64
        or not valid_opaque(binding["operation_id"])
        or not isinstance(binding["plan_hash"], str)
        or re.fullmatch(r"[0-9a-f]{64}", binding["plan_hash"]) is None
        or not valid_opaque(key.get("ha_installation"))
        or key.get("ha_installation") != installation
        or not valid_opaque(key.get("registry_entity_id"))
        or not valid_opaque(key.get("writer_scope"))
        or not isinstance(key.get("target"), str)
        or key["target"].count("/") != 1
        or not all(valid_opaque(part) for part in key["target"].split("/"))
        or binding["native_device"] != key["target"].split("/", 1)[0]
        or plan.get("direction") not in {"to_moli", "to_ha"}
        or binding.get("phase") not in {
            "ha", "prepared_to_moli", "ha_stopped", "moli", "prepared_to_ha",
            "moli_stopped", "reconcile_required",
        }
        or not isinstance(binding["reconcile_required"], bool)
        or binding["reconcile_required"] != (binding["phase"] == "reconcile_required")
    ):
        raise ProtocolError("migration_binding")
    return request


class QuiescenceVerifier:
    """Trusted observation hook. A base instance intentionally cannot prove quiescence."""

    def __init__(
        self,
        trusted_observer: Callable[[dict[str, Any]], QuiescenceObservation | None] | None = None,
    ) -> None:
        self._trusted_observer = trusted_observer

    def observe(self, binding: dict[str, Any]) -> QuiescenceObservation | None:
        if self._trusted_observer is None:
            return None
        observed = self._trusted_observer(binding)
        if not isinstance(observed, QuiescenceObservation):
            return None
        if not all(
            isinstance(value, bool)
            for value in (
                observed.disabled,
                observed.not_loaded,
                observed.no_pending,
                observed.scope_compatible,
                observed.registry_valid,
            )
        ) or observed.owner not in {"none", "moli", "ha", "unknown"}:
            return None
        return observed


def build_proof_message(
    request: dict[str, Any], observation: QuiescenceObservation | None,
    *, receiver_session: str,
) -> dict[str, Any] | None:
    binding = request["binding"]
    key = binding["key"]
    if (
        not valid_opaque(receiver_session)
        or observation is None
        or not proof_observation_complete(binding["phase"], observation)
    ):
        return None
    return {
        "v": 1,
        "installation": request["installation"],
        "session": receiver_session,
        "binding": key,
        "proof": {
            "operation": binding["operation_id"],
            "plan_hash": binding["plan_hash"],
            "writer_scope": key["writer_scope"],
            "revision": binding["revision"],
            "observed_owner": observation.owner,
            "quiescent": True,
        },
    }
