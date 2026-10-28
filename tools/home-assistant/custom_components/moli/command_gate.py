"""Pure bounded command admission for one Home Assistant MQTT session."""

from __future__ import annotations

from .protocol import MAX_U64


def resolve_registry_entry(entries, registry_entity_id: str):
    """Resolve HA's opaque registry UUID without confusing it with entity_id."""
    matches = [entry for entry in entries if getattr(entry, "id", None) == registry_entity_id]
    return matches[0] if len(matches) == 1 else None


class GenerationBook:
    """Bounded monotone generations learned only from current bridge frames."""

    def __init__(self, *, max_entries: int = 512) -> None:
        self.max_entries = max_entries
        self._values: dict[str, int] = {}
        self._overflow = False

    def announce(self, registry_entity_id: str, generation: int) -> bool:
        if (
            not registry_entity_id
            or not isinstance(generation, int)
            or isinstance(generation, bool)
            or generation < 0
            or generation > MAX_U64
        ):
            return False
        current = self._values.get(registry_entity_id)
        if current is None:
            if len(self._values) >= self.max_entries:
                self._overflow = True
                return False
            self._values[registry_entity_id] = generation
            return True
        if generation < current:
            return False
        self._values[registry_entity_id] = generation
        return True

    def accepts(self, registry_entity_id: str, generation: int) -> bool:
        current = self._values.get(registry_entity_id)
        if current is not None:
            return isinstance(generation, int) and not isinstance(generation, bool) and generation == current
        return (
            not self._overflow
            and isinstance(generation, int)
            and not isinstance(generation, bool)
            and generation == 0
        )


class AvailabilityTracker:
    """Monotonic peer liveness; retained births never renew the lease."""

    def __init__(self, *, timeout_seconds: float = 15.0) -> None:
        if timeout_seconds <= 0:
            raise ValueError("invalid availability timeout")
        self.timeout_seconds = timeout_seconds
        self.session: str | None = None
        self.last_live_at: float | None = None
        self.online = False

    def is_online(self, now: float) -> bool:
        return (
            self.online
            and self.last_live_at is not None
            and now - self.last_live_at < self.timeout_seconds
        )

    def observe(self, session: str, online: bool, *, retained: bool, now: float) -> bool:
        if retained or not session or type(online) is not bool:
            return False
        was_online = self.is_online(now)
        session_changed = self.session != session
        self.session = session
        self.last_live_at = now
        self.online = online
        return session_changed or (online and not was_online)

    def expire(self, now: float) -> bool:
        if self.online and not self.is_online(now):
            self.online = False
            return True
        return False

    def mark_offline(self) -> None:
        self.online = False


class CommandGate:
    """Track in-flight work and reject replayed request ids without a queue."""

    def __init__(self, *, max_pending: int = 16, max_seen: int = 256) -> None:
        if max_pending < 1 or max_seen < max_pending:
            raise ValueError("invalid command limits")
        self.max_pending = max_pending
        self.max_seen = max_seen
        self._pending: dict[str, str] = {}
        self._seen: set[str] = set()

    @property
    def pending_ids(self) -> set[str]:
        return set(self._pending)

    @property
    def seen_ids(self) -> set[str]:
        return set(self._seen)

    def begin(self, request_id: str, entity_id: str) -> bool:
        if (
            not request_id
            or not entity_id
            or request_id in self._seen
            or request_id in self._pending
            or entity_id in self._pending.values()
            or len(self._pending) >= self.max_pending
            or len(self._seen) >= self.max_seen
        ):
            return False
        self._seen.add(request_id)
        self._pending[request_id] = entity_id
        return True

    def finish(self, request_id: str) -> None:
        self._pending.pop(request_id, None)

    def clear_pending(self) -> None:
        self._pending.clear()
