//! Stable MQTT contract for a module, independent of its process group.
use crate::enum_catalogue::{CatalogRef, Choice, Pool};
use crate::{
    adapter::{Adapter, Observation},
    layout::{Kind as ModuleKind, Module},
};
use anyhow::{Context, Result, ensure};
use moli_core::{Device, EnumCodes, InstanceId, Kind, PointId, Value, validate};
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

fn text(value: &Value) -> Option<&str> {
    if let Value::Text(s) = value {
        Some(s)
    } else {
        None
    }
}

pub struct Instance {
    pub spec: Module,
    pub generation: String,
    devices: Vec<Device>,
    adapter: Box<dyn Adapter>,
    pool: Pool,
    enums: BTreeMap<PointId, CatalogRef>,
    last: BTreeMap<PointId, Value>,
    sequence: u64,
    calls: u64,
    processing_ns: u128,
    seeded: bool,
}

impl Instance {
    pub fn new(spec: Module) -> Result<Self> {
        use std::io::Read;
        let mut nonce = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let generation = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let id = InstanceId::from(spec.id.as_str());
        let mut adapter = match spec.kind {
            ModuleKind::Tahoma => crate::native::tahoma(&id)?,
            ModuleKind::Ariston => crate::native::ariston(&id)?,
            ModuleKind::Weather => crate::weather::build(&id)?,
            ModuleKind::AppleTv => crate::apple_tv::build(&id)?,
            ModuleKind::Music => crate::music::build(&id)?,
        };
        let mut devices = adapter.take_description();
        let mut pool = Pool::default();
        let mut enums = BTreeMap::new();
        for device in &mut devices {
            ensure!(device.instance == id, "adapter changed its module identity");
            for point in &mut device.points {
                if let Kind::Enum { values } = &point.kind {
                    let key = PointId::new(&device.id, &point.key);
                    let choices = values
                        .iter()
                        .map(|native| Choice {
                            native: native.to_string(),
                            label: adapter.enum_label(&key, native),
                        })
                        .collect::<Vec<_>>();
                    let reference = pool.update(None, &choices)?;
                    let mut codes = EnumCodes::empty();
                    for code in pool.active_codes(reference).context("catalogue missing")? {
                        codes.insert(code);
                    }
                    point.kind = Kind::EnumCode {
                        catalogue: reference.id,
                        version: reference.version,
                        codes,
                    };
                    enums.insert(key, reference);
                }
            }
        }
        Ok(Self {
            spec,
            generation,
            devices,
            adapter,
            pool,
            enums,
            last: BTreeMap::new(),
            sequence: 0,
            calls: 0,
            processing_ns: 0,
            seeded: false,
        })
    }
    pub fn topic(&self, suffix: &str) -> String {
        format!("moli-probe/module/{}/{suffix}", self.spec.id)
    }

    pub fn description(&self, group: &str) -> Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Description<'a> {
            v: u8,
            module: &'a str,
            module_kind: ModuleKind,
            group: &'a str,
            generation: &'a str,
            source: &'static str,
            execution: &'static str,
            revision: u32,
            devices: &'a [Device],
            catalogues: Vec<Json>,
            metadata: Json,
        }
        let refs: BTreeSet<_> = self.enums.values().copied().collect();
        let catalogues = refs
            .into_iter()
            .filter_map(|r| self.pool.descriptor(r))
            .collect();
        let bytes = serde_json::to_vec(&Description {
            v: 1,
            module: &self.spec.id,
            module_kind: self.spec.kind,
            group,
            generation: &self.generation,
            source: "fixtures",
            execution: "simulation",
            revision: 1,
            devices: &self.devices,
            catalogues,
            metadata: self.adapter.cold_metadata(),
        })?;
        ensure!(
            bytes.len() <= 256 * 1024,
            "module description exceeds bound"
        );
        Ok(bytes)
    }
    fn point(&self, id: &PointId) -> Option<&moli_core::PointSpec> {
        let (device, key) = id.split()?;
        self.devices.iter().find(|d| d.id == device)?.point(key)
    }
    pub fn observe(&mut self, step: u64) -> Result<Vec<(String, Vec<u8>)>> {
        let started = Instant::now();
        let mut observations = self.adapter.observe(step)?;
        if !self.seeded {
            let seen: BTreeSet<_> = observations.iter().map(|o| o.point.clone()).collect();
            for device in &self.devices {
                for point in &device.points {
                    let id = PointId::new(&device.id, &point.key);
                    if point.access.read && !seen.contains(&id) {
                        observations.push(Observation {
                            point: id,
                            value: Value::Null,
                        });
                    }
                }
            }
            self.seeded = true;
        }
        let mut out = Vec::new();
        for Observation { point, value } in observations {
            let value = if let Some(reference) = self.enums.get(&point) {
                text(&value)
                    .and_then(|s| self.pool.encode(*reference, s))
                    .map_or(Value::Null, Value::EnumCode)
            } else {
                value
            };
            let spec = self
                .point(&point)
                .context("adapter published an undeclared point")?;
            ensure!(spec.access.read, "adapter published a command-only point");
            let value = if value == Value::Null {
                Value::Null
            } else if let (Kind::Text, Value::Text(s)) = (&spec.kind, &value) {
                ensure!(s.len() <= 4096, "text observation exceeds UTF-8 byte bound");
                value
            } else {
                validate(&spec.kind, &value).context("adapter value violates its schema")?
            };
            if self.last.get(&point) == Some(&value) {
                continue;
            }
            self.sequence += 1;
            let quality = if value == Value::Null {
                "unknown"
            } else {
                "simulated"
            };
            #[derive(Serialize)]
            struct Delta<'a> {
                v: u8,
                module: &'a str,
                point: &'a PointId,
                value: &'a Value,
                quality: &'static str,
                sequence: u64,
                description_revision: u32,
                generation: &'a str,
            }
            let bytes = serde_json::to_vec(&Delta {
                v: 1,
                module: &self.spec.id,
                point: &point,
                value: &value,
                quality,
                sequence: self.sequence,
                description_revision: 1,
                generation: &self.generation,
            })?;
            ensure!(bytes.len() <= 32 * 1024, "state exceeds byte bound");
            out.push((self.topic(&format!("state/{point}")), bytes));
            self.last.insert(point, value);
        }
        self.calls += 1;
        self.processing_ns += started.elapsed().as_nanos();
        Ok(out)
    }
    pub fn command(&self, point: &PointId, value: &Json) -> Result<Json> {
        let spec = self.point(point).context("unknown command point")?;
        ensure!(spec.access.write, "read-only point");
        let value =
            validate(&spec.kind, &Value::from_json(value)).context("invalid command value")?;
        let native = if let Value::EnumCode(code) = value {
            let reference = *self
                .enums
                .get(point)
                .context("enum command without catalogue")?;
            Value::Text(
                self.pool
                    .decode(reference, code)
                    .context("inactive enum code")?
                    .into(),
            )
        } else {
            value
        };
        let body = self.adapter.simulate_command(point, &native)?;
        ensure!(
            serde_json::to_vec(&body)?.len() <= 256,
            "native command exceeds bound"
        );
        Ok(json!({"status":"simulated","sent":false,"state_confirmed":false,"native":body}))
    }
    #[cfg(test)]
    pub fn demo_command(&self) -> Option<(PointId, Json)> {
        for device in &self.devices {
            for p in &device.points {
                if !p.access.write {
                    continue;
                }
                let value = match &p.kind {
                    Kind::Binary => json!(true),
                    Kind::EnumCode { codes, .. } => {
                        let code = (0..=255).find(|c| codes.contains(*c))?;
                        json!(code)
                    }
                    Kind::Numeric { min, max, .. } => {
                        json!(min.unwrap_or(0.0).max(max.map_or(0.0, |v| v.min(60.0))))
                    }
                    _ => continue,
                };
                return Some((PointId::new(&device.id, &p.key), value));
            }
        }
        None
    }
    pub fn metrics(&self) -> Json {
        json!({"module":self.spec.id,"kind":self.spec.kind,"devices":self.devices.len(),"points":self.devices.iter().map(|d|d.points.len()).sum::<usize>(),"observed_slots":self.last.len(),"value_container_bytes":self.last.len()*std::mem::size_of::<Value>(),"free_text_utf8_bytes":self.last.values().filter_map(text).map(str::len).sum::<usize>(),"catalogue":self.pool.stats(),"adapter":self.adapter.runtime_metrics(),"processing_calls":self.calls,"processing_mean_ns":if self.calls==0{None}else{Some(self.processing_ns/self.calls as u128)},"measurement":"logical counters and wall processing time; not per-module RSS or total heap"})
    }
}

pub fn availability(module: &str, generation: &str, online: bool) -> (String, Vec<u8>) {
    (
        format!("moli-probe/module/{module}/availability"),
        serde_json::to_vec(
            &json!({"v":1,"module":module,"online":online,"generation":generation,"execution":"simulation"}),
        )
        .expect("availability scalar"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_enums_become_codes_and_regrouping_does_not_rename_devices() {
        let config = Module {
            id: "heater".into(),
            kind: ModuleKind::Ariston,
            enabled: true,
            source_ref: "fixture".into(),
        };
        let mut a = Instance::new(config).unwrap();
        let x: Json = serde_json::from_slice(&a.description("one").unwrap()).unwrap();
        let y: Json = serde_json::from_slice(&a.description("two").unwrap()).unwrap();
        assert_eq!(x["devices"], y["devices"]);
        assert_eq!(x["catalogues"], y["catalogues"]);
        let updates = a.observe(0).unwrap();
        assert!(
            updates
                .iter()
                .any(|(_, p)| serde_json::from_slice::<Json>(p).unwrap()["value"].is_u64())
        );
        assert!(
            a.enums
                .keys()
                .any(|p| matches!(a.last.get(p), Some(Value::EnumCode(_))))
        );
        let (p, v) = a.demo_command().unwrap();
        let r = a.command(&p, &v).unwrap();
        assert_eq!(r["sent"], false);
        assert_eq!(
            a.last.len(),
            a.metrics()["observed_slots"].as_u64().unwrap() as usize
        );
    }
}
