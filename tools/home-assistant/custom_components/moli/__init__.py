"""Home Assistant companion for the Moli MQTT bridge."""

from __future__ import annotations

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import ConfigEntryNotReady

from .bridge import MoliBridge
from .const import DOMAIN

PLATFORMS = [Platform.COVER, Platform.BUTTON]


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    bridge = MoliBridge(hass, entry)
    try:
        await bridge.async_start()
    except Exception as err:
        await bridge.async_stop()
        raise ConfigEntryNotReady("MQTT is not available for the Moli companion") from err
    hass.data.setdefault(DOMAIN, {})[entry.entry_id] = bridge
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    if not await hass.config_entries.async_unload_platforms(entry, PLATFORMS):
        return False
    bridge: MoliBridge = hass.data[DOMAIN].pop(entry.entry_id)
    await bridge.async_stop()
    if not hass.data[DOMAIN]:
        hass.data.pop(DOMAIN)
    return True
