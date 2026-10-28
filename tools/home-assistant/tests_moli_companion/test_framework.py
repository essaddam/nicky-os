"""Companion wiring against real Home Assistant classes, with MQTT I/O recorded only."""
from __future__ import annotations

import asyncio
import importlib
import json
from pathlib import Path
import sys
import tempfile
import time
from types import MappingProxyType, SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

try:
    from homeassistant.components import mqtt
    from homeassistant.components.cover import CoverEntityFeature
    from homeassistant.config_entries import ConfigEntries, ConfigEntry
    from homeassistant.core import HomeAssistant, callback
    from homeassistant.helpers import device_registry as dr, entity_registry as er, frame
except ModuleNotFoundError as error:
    if error.name != 'homeassistant':
        raise
    FRAMEWORK_AVAILABLE = False
else:
    FRAMEWORK_AVAILABLE = True

HERE = Path(__file__).resolve().parent
HA_TOOLS = HERE.parent
COMPONENT = HA_TOOLS / 'custom_components/moli'
WIRE = json.loads((HERE / 'fixtures/v1.json').read_text())


def companion_modules():
    """Do not let the pure tests' standalone sys.path hide broken package imports."""
    previous_path = list(sys.path)
    aliases = {}
    for name in ('protocol', 'inventory', 'command_gate', 'migration'):
        module = sys.modules.get(name)
        filename = getattr(module, '__file__', '')
        if filename and Path(filename).parent == COMPONENT:
            aliases[name] = sys.modules.pop(name)
    sys.path[:] = [str(HA_TOOLS), *[path for path in sys.path
                                   if Path(path or '.').resolve() != COMPONENT]]
    try:
        return (importlib.import_module('custom_components.moli.bridge'),
                importlib.import_module('custom_components.moli.cover'))
    finally:
        sys.path[:] = previous_path
        sys.modules.update(aliases)


@unittest.skipUnless(FRAMEWORK_AVAILABLE, 'Home Assistant framework is not installed')
class FrameworkContractTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.bridge = None
        self.hass = None
        self.temporary = tempfile.TemporaryDirectory(prefix='moli-framework-fixture-')
        self.addCleanup(self.temporary.cleanup)
        self.bridge_module, self.cover_module = companion_modules()
        self.hass = HomeAssistant(self.temporary.name)
        self.addAsyncCleanup(self._async_cleanup)
        frame.async_setup(self.hass)
        # Populate real registries without async_add, which would set up an integration.
        self.hass.config_entries = ConfigEntries(self.hass, {})
        self.source_entry = ConfigEntry(
            version=1, minor_version=1, domain='fixture', title='Fixture station',
            data={}, options={}, source='user', unique_id='framework-fixture',
            discovery_keys=MappingProxyType({}), subentries_data=None,
        )
        self.hass.config_entries._entries[self.source_entry.entry_id] = self.source_entry
        dr.async_setup(self.hass)
        await dr.async_load(self.hass, load_empty=True)
        await er.async_load(self.hass, load_empty=True)
        self.published = []
        self.handlers = {}
        self.unsubscribed = []
        self.on_publish = None

        async def publish(hass, topic, payload, *, qos, retain, **_kwargs):
            self.assertIs(hass, self.hass)
            value = json.loads(payload) if payload else None
            self.published.append({'topic': topic, 'value': value, 'qos': qos, 'retain': retain})
            if self.on_publish is not None:
                await self.on_publish(topic, value)

        async def subscribe(hass, topic, handler, *, qos, **_kwargs):
            self.assertIs(hass, self.hass)
            self.assertEqual(qos, 1)
            self.assertNotIn(topic, self.handlers)
            self.handlers[topic] = handler
            return lambda: self.unsubscribed.append(topic)

        for patcher in (
            patch.object(mqtt, 'async_wait_for_mqtt_client', AsyncMock(return_value=True)),
            patch.object(mqtt, 'async_publish', side_effect=publish),
            patch.object(mqtt, 'async_subscribe', side_effect=subscribe),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)
        self.entry = SimpleNamespace(entry_id='moli-framework-fixture', data={
            'installation': WIRE['ha_availability']['installation'],
            'prefix': 'moli-probe/framework-fixture',
        })
        self.bridge = self.bridge_module.MoliBridge(self.hass, self.entry)

    async def _async_cleanup(self):
        if self.bridge is not None:
            await self.bridge.async_stop()
        if self.hass is not None:
            await self.hass.async_block_till_done()
            await self.hass.async_stop(force=True)

    def messages(self, suffix):
        topic = self.bridge.topic(suffix)
        return [row for row in self.published if row['topic'] == topic]

    def message(self, suffix, payload, *, retained=False):
        return SimpleNamespace(topic=self.bridge.topic(suffix),
                               payload=json.dumps(payload).encode(), retain=retained)

    async def receive(self, subscription, suffix, payload, *, retained=False):
        self.handlers[self.bridge.topic(subscription)](
            self.message(suffix, payload, retained=retained))
        await self.hass.async_block_till_done()

    async def moli_online(self):
        await self.receive('moli/availability', 'moli/availability', WIRE['moli_availability'])

    def register(self, domain, unique_id, *, device=None, device_class=None, platform='fixture'):
        return er.async_get(self.hass).async_get_or_create(
            domain, platform, unique_id, config_entry=self.source_entry,
            device_id=device.id if device else None,
            suggested_object_id='fixture_' + unique_id,
            original_device_class=device_class, original_name='Fixture ' + unique_id,
        )

    async def test_start_subscribes_to_ha_commands_and_publishes_its_own_live_nonce(self):
        await self.bridge.async_start()
        expected = {self.bridge.topic(suffix) for suffix in (
            'moli/availability', 'moli/exports', 'moli/state/#', 'moli/result/#',
            'ha/command', 'moli/migration/request')}
        self.assertEqual(set(self.handlers), expected)
        births = self.messages('ha/availability')
        self.assertEqual(len(births), 1)
        self.assertEqual(births[0]['value'], {
            'v': 1, 'installation': self.bridge.installation,
            'session': self.bridge.session, 'online': True})
        self.assertFalse(births[0]['retain'])
        self.assertTrue(self.messages('ha/inventory'))
        await self.bridge.async_stop()
        self.assertEqual(set(self.unsubscribed), expected)
        self.assertEqual(self.messages('ha/availability')[-1]['value']['online'], False)

    async def test_late_live_handshake_responds_once_per_nonce_and_never_to_retained_birth(self):
        await self.bridge.async_start()
        await self.receive('moli/availability', 'moli/availability',
                           WIRE['moli_availability'], retained=True)
        self.assertEqual(len(self.messages('ha/availability')), 1)
        await self.moli_online()
        self.assertEqual(len(self.messages('ha/availability')), 2)
        self.assertEqual(self.messages('ha/availability')[-1]['value']['session'], self.bridge.session)
        for _ in range(3):
            await self.moli_online()
        self.assertEqual(len(self.messages('ha/availability')), 2, 'handshake echo must not loop')
        next_birth = dict(WIRE['moli_availability'], session='moli-new-fixture-session')
        await self.receive('moli/availability', 'moli/availability', next_birth)
        self.assertEqual(len(self.messages('ha/availability')), 3)
        self.assertTrue(all(row['retain'] is False for row in self.messages('ha/availability')))

    async def test_real_registries_publish_independent_sensor_channels_and_enum_codes(self):
        device = dr.async_get(self.hass).async_get_or_create(
            config_entry_id=self.source_entry.entry_id,
            identifiers={('fixture', 'station')}, name='Fixture station')
        temperature = self.register('sensor', 'temperature', device=device, device_class='temperature')
        humidity = self.register('sensor', 'humidity', device=device, device_class='humidity')
        mode = self.register('select', 'mode', device=device)
        proxy = self.register('sensor', 'excluded_proxy', device=device, platform='moli')
        self.hass.states.async_set(temperature.entity_id, '21.5',
                                   {'unit_of_measurement': '°C', 'device_class': 'temperature'})
        self.hass.states.async_set(humidity.entity_id, '43',
                                   {'unit_of_measurement': '%', 'device_class': 'humidity'})
        self.hass.states.async_set(mode.entity_id, 'eco', {'options': ['comfort', 'eco']})
        self.hass.states.async_set(proxy.entity_id, '99', {'unit_of_measurement': '°C'})
        await self.hass.async_block_till_done()
        await self.bridge.async_start()
        catalogue = self.messages('ha/inventory')[-1]['value']
        self.assertEqual(len(catalogue['devices']), 1)
        self.assertEqual(catalogue['devices'][0]['name'], 'Fixture station')
        channels = {row['registry_entity_id']: row for row in catalogue['devices'][0]['channels']}
        self.assertEqual(set(channels), {temperature.id, humidity.id, mode.id})
        self.assertNotEqual(channels[temperature.id]['id'], channels[humidity.id]['id'])
        self.assertEqual(channels[temperature.id]['semantic'], 'temperature')
        self.assertEqual(channels[humidity.id]['semantic'], 'humidity')
        device_id = catalogue['devices'][0]['id']
        def values_for(entry):
            return self.messages(f'ha/state/{device_id}/{channels[entry.id]["id"]}')
        self.assertEqual(values_for(temperature)[-1]['value']['value'], 21.5)
        self.assertEqual(values_for(humidity)[-1]['value']['value'], 43)
        kind = channels[mode.id]['kind']
        self.assertEqual(kind['type'], 'enum_code')
        encoded_mode = values_for(mode)[-1]['value']['value']
        self.assertIs(type(encoded_mode), int)
        self.assertEqual(next(row['native'] for row in kind['choices']
                              if row['code'] == encoded_mode), 'eco')
        humidity_messages = len(values_for(humidity))
        self.hass.states.async_set(temperature.entity_id, '22.0',
                                   {'unit_of_measurement': '°C', 'device_class': 'temperature'})
        await self.hass.async_block_till_done()
        self.assertEqual(values_for(temperature)[-1]['value']['value'], 22)
        self.assertEqual(len(values_for(humidity)), humidity_messages)

    async def test_native_cover_proxy_routes_callback_result_without_inventing_position(self):
        await self.bridge.async_start()
        await self.moli_online()
        await self.receive('moli/exports', 'moli/exports', WIRE['exports'], retained=True)
        self.assertEqual(len(self.bridge.exports_snapshot), 1)
        cover = self.cover_module.MoliCover(self.bridge, self.bridge.exports_snapshot[0])
        cover.hass = self.hass
        cover.entity_id = 'cover.fixture_proxy'
        self.assertFalse(cover.available)
        unavailable = dict(WIRE['cover_state'], available=False)
        suffix = 'moli/state/' + cover._item.binding.digest
        await self.receive('moli/state/#', suffix, unavailable, retained=True)
        self.assertFalse(cover.available)
        await self.receive('moli/state/#', suffix, dict(unavailable, available=True))
        self.assertTrue(cover.available)
        self.assertIsNone(cover.current_cover_position)
        self.assertIsNone(cover.is_closed)

        async def acknowledge(topic, value):
            if topic != self.bridge.topic('moli/command'):
                return
            self.assertEqual(value['session'], WIRE['moli_availability']['session'])
            self.assertEqual(value['binding'], WIRE['binding'])
            self.assertEqual(value['action'], 'open')
            self.assertNotIn('origin', value)
            current_ms = time.time_ns() // 1_000_000
            self.assertGreater(value['expires_at_ms'], current_ms)
            self.assertLessEqual(value['expires_at_ms'], current_ms + 5000)
            result = {'v': 1, 'installation': self.bridge.installation,
                      'request_id': value['request_id'], 'code': 'accepted'}
            callback_result = self.handlers[self.bridge.topic('moli/result/#')]
            suffix = 'moli/result/' + value['request_id']
            callback_result(self.message(suffix, result, retained=True))
            self.assertFalse(self.bridge._pending_moli[value['request_id']].done())
            callback_result(self.message(suffix, result))

        self.on_publish = acknowledge
        await asyncio.wait_for(cover.async_open_cover(), timeout=1)
        commands = self.messages('moli/command')
        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0]['qos'], 0)
        self.assertFalse(commands[0]['retain'])
        self.assertFalse(self.bridge._pending_moli)
        self.assertIsNone(cover.current_cover_position)
        self.assertIsNone(cover.is_closed)

    async def test_subscribed_ha_command_uses_real_registry_and_service_dispatch(self):
        native = self.register('cover', 'native_cover')
        features = int(CoverEntityFeature.OPEN | CoverEntityFeature.CLOSE | CoverEntityFeature.STOP)
        self.hass.states.async_set(native.entity_id, 'open', {'supported_features': features})
        calls = []

        @callback
        def close_cover(call):
            calls.append(dict(call.data))

        self.hass.services.async_register('cover', 'close_cover', close_cover)
        await self.hass.async_block_till_done()
        await self.bridge.async_start()
        await self.moli_online()
        command = {'v': 1, 'installation': self.bridge.installation, 'session': self.bridge.session,
                   'request_id': 'fixture-ha-request', 'entity_registry_id': native.id,
                   'generation': 0, 'expires_at_ms': time.time_ns() // 1_000_000 + 5000, 'action': 'close'}
        await self.receive('ha/command', 'ha/command', command)
        self.assertEqual(calls, [{'entity_id': native.entity_id}])
        results = self.messages('ha/result/fixture-ha-request')
        self.assertEqual(results[-1]['value']['code'], 'accepted')
        self.assertFalse(results[-1]['retain'])
        self.assertEqual(self.hass.states.get(native.entity_id).state, 'open',
                         'service admission is not physical feedback')


if __name__ == '__main__':
    unittest.main()
