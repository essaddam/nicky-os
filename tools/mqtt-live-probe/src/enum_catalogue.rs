//! Cold enum dictionaries. Hot values carry only a reference and a u8 code.
//! Catalogue versions are immutable; removed codes never name another native value.
use anyhow::{Context, Result, ensure};
use serde::de::{self, DeserializeOwned, SeqAccess, Visitor};
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value as Json, json};
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

const MAX_CODES: usize = 256;
const MAX_CATALOGUES: usize = 2048;
const MAX_TEXT_BYTES: usize = 256;
const MAX_CATALOGUE_BYTES: usize = 65536;
const MAX_PACKED_BYTES: usize = 4 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CatalogRef {
    pub id: u32,
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    pub native: String,
    pub label: String,
}

#[derive(Clone, Copy)]
struct Span {
    offset: u16,
    len: u16,
}

struct Entry {
    native: Span,
    label: Span,
    active: bool,
}

struct Catalogue {
    reference: CatalogRef,
    entries: Vec<Entry>,
    packed: Vec<u8>,
    json_bytes: usize,
}

impl Catalogue {
    fn new(reference: CatalogRef) -> Self {
        Self {
            reference,
            entries: Vec::new(),
            packed: Vec::new(),
            json_bytes: 0,
        }
    }

    fn text(&self, span: Span) -> &str {
        let start = usize::from(span.offset);
        // Spans are created exclusively by intern(), which copies complete UTF-8 strings.
        std::str::from_utf8(&self.packed[start..start + usize::from(span.len)])
            .expect("interned UTF-8")
    }

    fn native_index(&self, native: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| self.text(entry.native) == native)
    }

    fn intern(&mut self, text: &str) -> Result<Span> {
        ensure!(text.len() <= MAX_TEXT_BYTES, "enum text exceeds byte bound");
        if text.is_empty() {
            return Ok(Span { offset: 0, len: 0 });
        }
        for entry in &self.entries {
            for span in [entry.native, entry.label] {
                if self.text(span) == text {
                    return Ok(span);
                }
            }
        }
        ensure!(
            self.packed.len() + text.len() <= MAX_CATALOGUE_BYTES,
            "enum packed catalogue exceeds offset bound"
        );
        let span = Span {
            offset: u16::try_from(self.packed.len()).context("enum offset exceeds bound")?,
            len: u16::try_from(text.len()).context("enum text exceeds bound")?,
        };
        self.packed.extend_from_slice(text.as_bytes());
        Ok(span)
    }

    fn push(&mut self, native: &str, label: &str, active: bool) -> Result<()> {
        ensure!(
            self.entries.len() < MAX_CODES,
            "enum code history exhausted"
        );
        let native_span = self.intern(native)?;
        let label_span = if native == label {
            native_span
        } else {
            self.intern(label)?
        };
        self.entries.push(Entry {
            native: native_span,
            label: label_span,
            active,
        });
        Ok(())
    }

    fn compact(&mut self) -> Result<()> {
        self.entries.shrink_to_fit();
        self.packed.shrink_to_fit();
        self.json_bytes = serde_json::to_vec(self)?.len();
        Ok(())
    }

    fn matches_choices(&self, choices: &[Choice]) -> bool {
        self.entries.iter().filter(|entry| entry.active).count() == choices.len()
            && choices.iter().all(|choice| {
                self.native_index(&choice.native).is_some_and(|index| {
                    let entry = &self.entries[index];
                    entry.active && self.text(entry.label) == choice.label
                })
            })
    }

    fn same_history(&self, other: &Self) -> bool {
        self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .zip(&other.entries)
                .all(|(left, right)| {
                    left.active == right.active
                        && self.text(left.native) == other.text(right.native)
                        && self.text(left.label) == other.text(right.label)
                })
    }
}

#[derive(Serialize)]
struct EntryView<'a> {
    code: u8,
    native: &'a str,
    label: &'a str,
    active: bool,
}

impl Serialize for Catalogue {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let entries: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .map(|(code, entry)| EntryView {
                code: code as u8,
                native: self.text(entry.native),
                label: self.text(entry.label),
                active: entry.active,
            })
            .collect();
        let mut object = serializer.serialize_struct("Catalogue", 3)?;
        object.serialize_field("id", &self.reference.id)?;
        object.serialize_field("version", &self.reference.version)?;
        object.serialize_field("entries", &entries)?;
        object.end()
    }
}

#[derive(Clone)]
pub(super) struct Pool {
    catalogues: Vec<Arc<Catalogue>>,
    next_id: u32,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            catalogues: Vec::new(),
            next_id: 1,
        }
    }
}

#[derive(Serialize)]
struct SnapshotView<'a> {
    format: u32,
    next_id: u32,
    catalogues: CataloguesView<'a>,
}

struct CataloguesView<'a>(&'a [Arc<Catalogue>]);
impl Serialize for CataloguesView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for catalogue in self.0 {
            sequence.serialize_element(catalogue.as_ref())?;
        }
        sequence.end()
    }
}

impl Pool {
    fn catalogue(&self, reference: CatalogRef) -> Option<&Catalogue> {
        self.catalogues
            .iter()
            .find(|catalogue| catalogue.reference == reference)
            .map(Arc::as_ref)
    }

    fn snapshot_bytes(&self, next_id: u32, extra: Option<&Catalogue>) -> usize {
        let count = self.catalogues.len() + usize::from(extra.is_some());
        let empty = json!({"format":1,"next_id":next_id,"catalogues":[]})
            .to_string()
            .len();
        empty
            + count.saturating_sub(1)
            + self
                .catalogues
                .iter()
                .map(|catalogue| catalogue.json_bytes)
                .sum::<usize>()
            + extra.map_or(0, |catalogue| catalogue.json_bytes)
    }

    pub(super) fn update(
        &mut self,
        previous: Option<CatalogRef>,
        choices: &[Choice],
    ) -> Result<CatalogRef> {
        ensure!(choices.len() <= MAX_CODES, "too many enum choices");
        for (index, choice) in choices.iter().enumerate() {
            ensure!(
                choice.native.len() <= MAX_TEXT_BYTES && choice.label.len() <= MAX_TEXT_BYTES,
                "enum text exceeds byte bound"
            );
            ensure!(
                !choices[..index]
                    .iter()
                    .any(|old| old.native == choice.native),
                "duplicate enum native value"
            );
        }
        let old = previous
            .map(|reference| {
                self.catalogue(reference)
                    .context("unknown enum catalogue reference")
            })
            .transpose()?;
        if let Some(old) = old
            && old.matches_choices(choices)
        {
            return Ok(old.reference);
        }
        if old.is_none()
            && let Some(shared) = self
                .catalogues
                .iter()
                .find(|catalogue| catalogue.matches_choices(choices))
        {
            return Ok(shared.reference);
        }
        let version = old
            .map_or(Some(1), |old| old.reference.version.checked_add(1))
            .context("enum version exhausted")?;
        let mut candidate = Catalogue::new(CatalogRef {
            id: self.next_id,
            version,
        });
        if let Some(old) = old {
            for entry in &old.entries {
                let native = old.text(entry.native);
                let choice = choices.iter().find(|choice| choice.native == native);
                candidate.push(
                    native,
                    choice.map_or_else(|| old.text(entry.label), |choice| choice.label.as_str()),
                    choice.is_some(),
                )?;
            }
        }
        for choice in choices {
            if candidate.native_index(&choice.native).is_none() {
                candidate.push(&choice.native, &choice.label, true)?;
            }
        }
        if let Some(shared) = self
            .catalogues
            .iter()
            .find(|catalogue| catalogue.same_history(&candidate))
        {
            return Ok(shared.reference);
        }
        ensure!(
            self.catalogues.len() < MAX_CATALOGUES,
            "enum catalogue count bound exceeded"
        );
        ensure!(
            self.catalogues
                .iter()
                .map(|catalogue| catalogue.packed.len())
                .sum::<usize>()
                + candidate.packed.len()
                <= MAX_PACKED_BYTES,
            "enum pool byte bound exceeded"
        );
        let next_id = self
            .next_id
            .checked_add(1)
            .context("enum identity space exhausted")?;
        candidate.compact()?;
        ensure!(
            self.snapshot_bytes(next_id, Some(&candidate)) <= MAX_SNAPSHOT_BYTES,
            "enum snapshot byte bound exceeded"
        );
        let reference = candidate.reference;
        self.catalogues.push(Arc::new(candidate));
        self.next_id = next_id;
        Ok(reference)
    }

    pub(super) fn encode(&self, reference: CatalogRef, native: &str) -> Option<u8> {
        let catalogue = self.catalogue(reference)?;
        let index = catalogue.native_index(native)?;
        catalogue.entries[index].active.then_some(index as u8)
    }

    pub(super) fn decode(&self, reference: CatalogRef, code: u8) -> Option<&str> {
        let catalogue = self.catalogue(reference)?;
        let entry = catalogue.entries.get(usize::from(code))?;
        entry.active.then(|| catalogue.text(entry.native))
    }

    pub(super) fn active_codes(&self, reference: CatalogRef) -> Option<Vec<u8>> {
        Some(
            self.catalogue(reference)?
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.active)
                .map(|(code, _)| code as u8)
                .collect(),
        )
    }

    pub(super) fn descriptor(&self, reference: CatalogRef) -> Option<Json> {
        serde_json::to_value(self.catalogue(reference)?).ok()
    }

    pub(super) fn retain_refs(&mut self, references: impl IntoIterator<Item = CatalogRef>) {
        let mut keep = vec![false; self.catalogues.len()];
        for reference in references {
            if let Some(index) = self
                .catalogues
                .iter()
                .position(|catalogue| catalogue.reference == reference)
            {
                keep[index] = true;
            }
        }
        let mut index = 0;
        self.catalogues.retain(|_| {
            let retain = keep[index];
            index += 1;
            retain
        });
        self.catalogues.shrink_to_fit();
    }

    pub(super) fn export(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(&SnapshotView {
            format: 1,
            next_id: self.next_id,
            catalogues: CataloguesView(&self.catalogues),
        })?;
        ensure!(
            bytes.len() <= MAX_SNAPSHOT_BYTES,
            "enum snapshot byte bound exceeded"
        );
        Ok(bytes)
    }

    pub(super) fn load(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_SNAPSHOT_BYTES,
            "enum snapshot byte bound exceeded"
        );
        let saved: SavedSnapshot =
            serde_json::from_slice(bytes).context("invalid enum snapshot")?;
        ensure!(
            saved.format == 1 && saved.next_id != 0,
            "unsupported enum snapshot"
        );
        let mut pool = Self {
            next_id: saved.next_id,
            catalogues: Vec::new(),
        };
        let mut packed_bytes = 0;
        for saved in saved.catalogues {
            ensure!(
                saved.id != 0 && saved.id < pool.next_id && saved.version != 0,
                "invalid enum catalogue identity"
            );
            ensure!(
                !pool
                    .catalogues
                    .iter()
                    .any(|catalogue| catalogue.reference.id == saved.id),
                "duplicate enum catalogue identity"
            );
            let mut catalogue = Catalogue::new(CatalogRef {
                id: saved.id,
                version: saved.version,
            });
            for (code, entry) in saved.entries.into_iter().enumerate() {
                ensure!(
                    usize::from(entry.code) == code,
                    "enum codes must be contiguous and unique"
                );
                ensure!(
                    catalogue.native_index(&entry.native).is_none(),
                    "duplicate enum native value"
                );
                catalogue.push(&entry.native, &entry.label, entry.active)?;
            }
            packed_bytes += catalogue.packed.len();
            ensure!(
                packed_bytes <= MAX_PACKED_BYTES,
                "enum pool byte bound exceeded"
            );
            catalogue.compact()?;
            pool.catalogues.push(Arc::new(catalogue));
        }
        ensure!(
            pool.snapshot_bytes(pool.next_id, None) <= MAX_SNAPSHOT_BYTES,
            "enum snapshot byte bound exceeded"
        );
        pool.catalogues.shrink_to_fit();
        Ok(pool)
    }

    pub(super) fn stats(&self) -> Json {
        let packed: usize = self
            .catalogues
            .iter()
            .map(|catalogue| catalogue.packed.len())
            .sum();
        let packed_capacity: usize = self
            .catalogues
            .iter()
            .map(|catalogue| catalogue.packed.capacity())
            .sum();
        let entries: usize = self
            .catalogues
            .iter()
            .map(|catalogue| catalogue.entries.len())
            .sum();
        let entry_capacity: usize = self
            .catalogues
            .iter()
            .map(|catalogue| catalogue.entries.capacity() * std::mem::size_of::<Entry>())
            .sum();
        let overhead = entry_capacity
            + self.catalogues.capacity() * std::mem::size_of::<Arc<Catalogue>>()
            + self.catalogues.len() * std::mem::size_of::<Catalogue>()
            + std::mem::size_of::<Self>();
        json!({"catalogues":self.catalogues.len(),"active_choices":self.catalogues.iter().flat_map(|catalogue|&catalogue.entries).filter(|entry|entry.active).count(),"codes":entries,"packed_bytes":packed,"packed_capacity_bytes":packed_capacity,"entry_bytes":entries*std::mem::size_of::<Entry>(),"entry_capacity_bytes":entry_capacity,"entry_struct_bytes":std::mem::size_of::<Entry>(),"catalogue_struct_bytes":std::mem::size_of::<Catalogue>(),"catalogue_capacity":self.catalogues.capacity(),"overhead_bytes":overhead,"owned_capacity_bytes":packed_capacity+overhead,"snapshot_bytes":self.snapshot_bytes(self.next_id,None),"shared_tables":self.catalogues.iter().filter(|catalogue|Arc::strong_count(catalogue)>1).count(),"measurement":"logical_table_bytes_and_vec_capacities_excludes_allocator_and_arc_headers_or_rss"})
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSnapshot {
    format: u32,
    next_id: u32,
    #[serde(deserialize_with = "read_catalogues")]
    catalogues: Vec<SavedCatalogue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedCatalogue {
    id: u32,
    version: u32,
    #[serde(deserialize_with = "read_entries")]
    entries: Vec<SavedEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedEntry {
    code: u8,
    #[serde(deserialize_with = "read_text")]
    native: String,
    #[serde(deserialize_with = "read_text")]
    label: String,
    active: bool,
}

fn read_text<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<String, D::Error> {
    struct TextVisitor;
    impl Visitor<'_> for TextVisitor {
        type Value = String;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded enum text")
        }
        fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<String, E> {
            if value.len() > MAX_TEXT_BYTES {
                return Err(E::custom("enum text exceeds byte bound"));
            }
            Ok(value.into())
        }
    }
    deserializer.deserialize_str(TextVisitor)
}

fn bounded_rows<'de, D: Deserializer<'de>, T: DeserializeOwned, const N: usize>(
    deserializer: D,
) -> std::result::Result<Vec<T>, D::Error> {
    struct Rows<T, const N: usize>(PhantomData<T>);
    impl<'de, T: DeserializeOwned, const N: usize> Visitor<'de> for Rows<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("bounded enum rows")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> std::result::Result<Vec<T>, A::Error> {
            let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(N));
            loop {
                if values.len() == N {
                    if sequence.next_element::<de::IgnoredAny>()?.is_some() {
                        return Err(de::Error::custom("too many enum rows"));
                    }
                    return Ok(values);
                }
                let Some(value) = sequence.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
        }
    }
    deserializer.deserialize_seq(Rows::<T, N>(PhantomData))
}

fn read_catalogues<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<SavedCatalogue>, D::Error> {
    bounded_rows::<D, SavedCatalogue, MAX_CATALOGUES>(deserializer)
}
fn read_entries<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<SavedEntry>, D::Error> {
    bounded_rows::<D, SavedEntry, MAX_CODES>(deserializer)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn choices(values: &[&str]) -> Vec<Choice> {
        values
            .iter()
            .map(|value| Choice {
                native: (*value).into(),
                label: (*value).into(),
            })
            .collect()
    }

    #[test]
    fn reordering_and_identical_sets_share_a_reference_and_keep_codes() {
        let mut pool = Pool::default();
        let first = pool
            .update(None, &choices(&["off", "heat", "auto"]))
            .unwrap();
        let reordered = choices(&["auto", "off", "heat"]);
        assert_eq!(pool.update(Some(first), &reordered).unwrap(), first);
        assert_eq!(pool.update(None, &reordered).unwrap(), first);
        assert_eq!(pool.encode(first, "off"), Some(0));
        assert_eq!(pool.encode(first, "auto"), Some(2));
        assert_eq!(pool.decode(first, 1), Some("heat"));
        assert_eq!(pool.encode(first, "unknown"), None);
        assert_eq!(pool.decode(first, 255), None);
        assert_eq!(pool.stats()["catalogues"], 1);
    }

    #[test]
    fn removals_labels_and_appends_fork_immutable_history_without_reassigning_codes() {
        let mut pool = Pool::default();
        let first = pool
            .update(None, &choices(&["off", "heat", "auto"]))
            .unwrap();
        let before = pool.descriptor(first).unwrap();
        let changed = pool
            .update(
                Some(first),
                &[
                    Choice {
                        native: "heat".into(),
                        label: "Chauffer".into(),
                    },
                    Choice {
                        native: "cool".into(),
                        label: "Refroidir".into(),
                    },
                ],
            )
            .unwrap();
        assert_ne!(changed.id, first.id);
        assert_eq!(changed.version, first.version + 1);
        assert_eq!(pool.descriptor(first).unwrap(), before);
        assert_eq!(pool.encode(first, "off"), Some(0));
        assert_eq!(pool.encode(changed, "heat"), Some(1));
        assert_eq!(pool.encode(changed, "cool"), Some(3));
        assert_eq!(pool.encode(changed, "off"), None);
        assert_eq!(pool.decode(changed, 0), None);
        let descriptor = pool.descriptor(changed).unwrap();
        assert_eq!(
            descriptor["entries"][0],
            json!({"code":0,"native":"off","label":"off","active":false})
        );
        assert_eq!(descriptor["entries"][1]["label"], "Chauffer");
        assert_eq!(pool.active_codes(changed), Some(vec![1, 3]));
        let same = pool
            .update(
                Some(first),
                &[
                    Choice {
                        native: "cool".into(),
                        label: "Refroidir".into(),
                    },
                    Choice {
                        native: "heat".into(),
                        label: "Chauffer".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(same, changed);
        let reactivated = pool
            .update(Some(changed), &choices(&["off", "cool"]))
            .unwrap();
        assert_eq!(pool.encode(reactivated, "off"), Some(0));
        assert_eq!(pool.encode(reactivated, "cool"), Some(3));
    }

    #[test]
    fn numeric_native_strings_are_not_coerced_and_utf8_labels_remain_exact() {
        let mut pool = Pool::default();
        let reference = pool
            .update(
                None,
                &[
                    Choice {
                        native: "01".into(),
                        label: "Éco 🌡".into(),
                    },
                    Choice {
                        native: "1".into(),
                        label: "Confort".into(),
                    },
                    Choice {
                        native: "1.0".into(),
                        label: "1.0".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(pool.encode(reference, "01"), Some(0));
        assert_eq!(pool.encode(reference, "1"), Some(1));
        assert_eq!(pool.encode(reference, "1.0"), Some(2));
        assert_eq!(
            pool.descriptor(reference).unwrap()["entries"][0]["label"],
            "Éco 🌡"
        );
        assert_eq!(pool.decode(reference, 0), Some("01"));
    }

    #[test]
    fn restart_and_garbage_collection_preserve_history_and_never_reuse_ids() {
        let mut pool = Pool::default();
        let first = pool.update(None, &choices(&["a", "b"])).unwrap();
        let second = pool.update(Some(first), &choices(&["b", "c"])).unwrap();
        let snapshot = pool.export().unwrap();
        let mut restored = Pool::load(&snapshot).unwrap();
        assert_eq!(restored.descriptor(first), pool.descriptor(first));
        assert_eq!(restored.descriptor(second), pool.descriptor(second));
        assert_eq!(
            restored
                .update(Some(second), &choices(&["c", "b"]))
                .unwrap(),
            second
        );
        restored.retain_refs([second]);
        assert!(restored.descriptor(first).is_none());
        assert!(restored.update(Some(first), &choices(&["x"])).is_err());
        let third = restored.update(None, &choices(&["new"])).unwrap();
        assert!(third.id > second.id);
        restored.retain_refs([]);
        let mut restored = Pool::load(&restored.export().unwrap()).unwrap();
        assert!(
            restored
                .update(None, &choices(&["after empty GC"]))
                .unwrap()
                .id
                > third.id
        );
    }

    #[test]
    fn code_and_utf8_bounds_reject_updates_without_mutating_the_pool() {
        let mut pool = Pool::default();
        let all: Vec<_> = (0..MAX_CODES)
            .map(|i| Choice {
                native: i.to_string(),
                label: i.to_string(),
            })
            .collect();
        let full = pool.update(None, &all).unwrap();
        assert_eq!(pool.encode(full, "255"), Some(255));
        let before = pool.export().unwrap();
        assert!(pool.update(Some(full), &choices(&["new"])).is_err());
        let mut excessive = all;
        excessive.push(Choice {
            native: "extra".into(),
            label: "extra".into(),
        });
        assert!(pool.update(None, &excessive).is_err());
        for value in [
            Choice {
                native: "é".repeat(129),
                label: "small".into(),
            },
            Choice {
                native: "small".into(),
                label: "é".repeat(129),
            },
        ] {
            assert!(pool.update(None, &[value]).is_err());
        }
        assert!(
            pool.update(None, &choices(&["duplicate", "duplicate"]))
                .is_err()
        );
        assert_eq!(pool.export().unwrap(), before);
        assert!(
            pool.update(
                None,
                &[Choice {
                    native: "é".repeat(128),
                    label: "🌡".repeat(64)
                }]
            )
            .is_ok()
        );
    }

    #[test]
    fn labels_equal_to_native_are_stored_once_and_stats_count_real_vec_capacity() {
        let mut pool = Pool::default();
        let reference = pool
            .update(None, &choices(&["off", "heat", "auto"]))
            .unwrap();
        assert_eq!(pool.stats()["packed_bytes"], 11);
        assert!(pool.stats()["owned_capacity_bytes"].as_u64().unwrap() >= 11);
        let changed = pool
            .update(
                Some(reference),
                &[
                    Choice {
                        native: "off".into(),
                        label: "auto".into(),
                    },
                    Choice {
                        native: "heat".into(),
                        label: "heat".into(),
                    },
                    Choice {
                        native: "auto".into(),
                        label: "auto".into(),
                    },
                ],
            )
            .unwrap();
        pool.retain_refs([changed]);
        assert_eq!(pool.stats()["packed_bytes"], 11);
    }

    #[test]
    fn clones_share_immutable_tables_and_version_exhaustion_still_allows_identical_updates() {
        let mut pool = Pool::default();
        let reference = pool.update(None, &choices(&["off", "heat"])).unwrap();
        let mut clone = pool.clone();
        assert!(Arc::ptr_eq(&pool.catalogues[0], &clone.catalogues[0]));
        clone
            .update(Some(reference), &choices(&["off", "cool"]))
            .unwrap();
        assert_eq!(pool.stats()["catalogues"], 1);
        assert_eq!(pool.encode(reference, "heat"), Some(1));
        let bytes=serde_json::to_vec(&json!({"format":1,"next_id":2,"catalogues":[{"id":1,"version":u32::MAX,"entries":[{"code":0,"native":"off","label":"off","active":true}]}]})).unwrap();
        let mut pool = Pool::load(&bytes).unwrap();
        let reference = CatalogRef {
            id: 1,
            version: u32::MAX,
        };
        assert_eq!(
            pool.update(Some(reference), &choices(&["off"])).unwrap(),
            reference
        );
        assert!(pool.update(Some(reference), &choices(&["new"])).is_err());
    }

    #[test]
    fn packed_offsets_allow_exact_u16_space_and_reject_one_more_label() {
        let mut pool = Pool::default();
        let mut values: Vec<_> = (0..MAX_CODES)
            .map(|code| {
                let value = format!("{code:03}{}", "x".repeat(253));
                Choice {
                    native: value.clone(),
                    label: value,
                }
            })
            .collect();
        let reference = pool.update(None, &values).unwrap();
        assert_eq!(pool.stats()["packed_bytes"], MAX_CATALOGUE_BYTES);
        assert_eq!(
            pool.decode(reference, 255),
            Some(values[255].native.as_str())
        );
        let before = pool.export().unwrap();
        values[0].label = "y".repeat(256);
        assert!(pool.update(Some(reference), &values).is_err());
        assert_eq!(pool.export().unwrap(), before);
    }

    #[test]
    fn catalogue_count_is_bounded_and_gc_does_not_recycle_identity() {
        let mut pool = Pool::default();
        for index in 0..MAX_CATALOGUES {
            pool.update(None, &choices(&[&format!("mode-{index}")]))
                .unwrap();
        }
        let before = pool.export().unwrap();
        assert!(pool.update(None, &choices(&["overflow"])).is_err());
        assert_eq!(pool.export().unwrap(), before);
        pool.retain_refs([]);
        assert!(pool.update(None, &choices(&["after GC"])).unwrap().id > MAX_CATALOGUES as u32);
    }

    #[test]
    fn every_successful_update_remains_exportable_within_the_snapshot_budget() {
        let mut pool = Pool::default();
        let mut rejected = false;
        for table in 0..64 {
            let values: Vec<_> = (0..MAX_CODES)
                .map(|code| {
                    let value = format!("{table:02}-{code:03}{}", "x".repeat(250));
                    Choice {
                        native: value.clone(),
                        label: value,
                    }
                })
                .collect();
            let before = pool.export().unwrap();
            if pool.update(None, &values).is_err() {
                assert_eq!(pool.export().unwrap(), before);
                rejected = true;
                break;
            }
            let exported = pool.export().unwrap();
            assert!(exported.len() <= MAX_SNAPSHOT_BYTES);
            assert_eq!(pool.stats()["snapshot_bytes"], exported.len());
        }
        assert!(rejected);
        assert!(pool.stats()["packed_bytes"].as_u64().unwrap() <= MAX_PACKED_BYTES as u64);
    }

    #[test]
    fn load_rejects_unknown_fields_duplicate_codes_ids_and_invalid_snapshots() {
        let mut pool = Pool::default();
        pool.update(None, &choices(&["off", "heat"])).unwrap();
        let original: Json = serde_json::from_slice(&pool.export().unwrap()).unwrap();
        for mutation in 0..8 {
            let mut value = original.clone();
            match mutation {
                0 => value["unknown"] = json!(true),
                1 => value["catalogues"][0]["entries"][1]["code"] = json!(0),
                2 => value["catalogues"][0]["entries"][1]["native"] = json!("off"),
                3 => value["next_id"] = json!(1),
                4 => value["catalogues"][0]["version"] = json!(0),
                5 => value["catalogues"][0]["entries"][0]["native"] = json!("x".repeat(257)),
                6 => {
                    let duplicate = value["catalogues"][0].clone();
                    value["catalogues"].as_array_mut().unwrap().push(duplicate);
                }
                _ => value["catalogues"][0]["entries"][0]["active"] = json!(1),
            }
            assert!(
                Pool::load(&serde_json::to_vec(&value).unwrap()).is_err(),
                "mutation {mutation}"
            );
        }
        assert!(Pool::load(&vec![b' '; MAX_SNAPSHOT_BYTES + 1]).is_err());
        assert!(Pool::load(b"{}").is_err());
    }
}
