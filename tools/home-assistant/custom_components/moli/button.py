"""Favorite action exposed separately from cover motion and position."""

from __future__ import annotations

from typing import Any

from homeassistant.components.button import ButtonEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import device_registry as dr
from homeassistant.helpers.dispatcher import async_dispatcher_connect
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .bridge import MoliBridge, SIGNAL_EXPORTS
from .const import DOMAIN
from .protocol import CoverExport


async def async_setup_entry(
    hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddEntitiesCallback
) -> None:
    bridge: MoliBridge = hass.data[DOMAIN][entry.entry_id]
    entities: dict[str, MoliFavoriteButton] = {}

    @callback
    def sync_exports() -> None:
        current = {item.binding.digest: item for item in bridge.favorite_exports}
        new: list[MoliFavoriteButton] = []
        for identity, entity in entities.items():
            entity.set_export(current.get(identity))
        for identity, item in current.items():
            if identity not in entities:
                entity = MoliFavoriteButton(bridge, item)
                entities[identity] = entity
                new.append(entity)
        if new:
            async_add_entities(new)

    sync_exports()
    unsubscribe = async_dispatcher_connect(
        hass, SIGNAL_EXPORTS.format(entry.entry_id), sync_exports
    )
    entry.async_on_unload(unsubscribe)


class MoliFavoriteButton(ButtonEntity):
    """One explicit favorite action; never represented as a cover position."""

    _attr_has_entity_name = True
    _attr_translation_key = "favorite"

    def __init__(self, bridge: MoliBridge, item: CoverExport) -> None:
        self.bridge = bridge
        self._item: CoverExport | None = item
        self._retired = False
        self._attr_unique_id = f"favorite-{item.binding.digest}"
        self._attr_name = item.label or None

    @property
    def device_info(self) -> dr.DeviceInfo:
        assert self._item is not None
        return dr.DeviceInfo(
            identifiers={(DOMAIN, self._item.binding.digest)},
            name=self._item.label or "Moli cover",
            manufacturer="Moli OS",
        )

    @property
    def available(self) -> bool:
        return (
            not self._retired
            and self._item is not None
            and self.bridge.export_is_current(self._item)
        )

    @callback
    def set_export(self, item: CoverExport | None) -> None:
        self._item = item
        self._retired = item is None
        self.async_write_ha_state()

    async def async_press(self) -> None:
        if self._item is None or self._retired:
            raise RuntimeError("Moli favorite export is no longer available")
        await self.bridge.async_send_cover_action(self._item, "my")
