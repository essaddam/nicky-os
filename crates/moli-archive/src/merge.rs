//! Four-way external merge. Source readers and temporary rows stay bounded.
use crate::{
    manifest::Part,
    parquet::{self, PartReader, PartWriter, Record, Selection},
};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
struct Run {
    files: std::vec::IntoIter<(PathBuf, Option<[u8; 32]>)>,
    reader: Option<PartReader>,
    selection: Option<Arc<Selection>>,
}
impl Run {
    fn next(&mut self) -> Result<Option<Record>> {
        loop {
            if let Some(reader) = &mut self.reader
                && let Some(r) = reader.next_record()?
            {
                return Ok(Some(r));
            }
            let Some((path, hash)) = self.files.next() else {
                return Ok(None);
            };
            if let Some(hash) = hash {
                ensure!(parquet::hash_file(&path)? == hash, "corrupt_merge_input");
            }
            self.reader = Some(PartReader::filtered(&path, self.selection.clone())?);
        }
    }
}
#[derive(Debug)]
struct Merge {
    runs: Vec<Run>,
    heads: Vec<Option<Record>>,
    resolve: bool,
    pending: Option<Record>,
}
impl Merge {
    fn new(mut runs: Vec<Run>, resolve: bool) -> Result<Self> {
        let heads = runs.iter_mut().map(Run::next).collect::<Result<Vec<_>>>()?;
        Ok(Self {
            runs,
            heads,
            resolve,
            pending: None,
        })
    }
    fn pull(&mut self) -> Result<Option<Record>> {
        let index = self
            .heads
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.as_ref().map(|r| (i, r)))
            .min_by(|a, b| a.1.key().cmp(&b.1.key()))
            .map(|(i, _)| i);
        let Some(index) = index else {
            return Ok(None);
        };
        let row = self.heads[index].take();
        self.heads[index] = self.runs[index].next()?;
        Ok(row)
    }
    fn next(&mut self) -> Result<Option<Record>> {
        while let Some(row) = self.pull()? {
            if let Some(previous) = &mut self.pending {
                let same = previous.entity == row.entity
                    && previous.ts == row.ts
                    && (self.resolve || previous.seq == row.seq);
                if same {
                    if previous.seq == row.seq {
                        ensure!(*previous == row, "conflicting_duplicate_row");
                    } else {
                        *previous = row;
                    }
                    continue;
                }
            }
            let emitted = self.pending.replace(row);
            if emitted.is_some() {
                return Ok(emitted);
            }
        }
        Ok(self.pending.take())
    }
}
#[derive(Debug)]
pub struct ExternalMerge {
    merge: Merge,
    temporary: Vec<PathBuf>,
}
impl ExternalMerge {
    pub fn new(
        root: &Path,
        parts: &[Part],
        selection: Option<&Arc<Selection>>,
        resolve: bool,
        prefix: &str,
    ) -> Result<Self> {
        let mut groups: BTreeMap<(String, String, i64, u8), Vec<&Part>> = BTreeMap::new();
        for p in parts {
            groups
                .entry((p.group.clone(), p.month.clone(), p.run, p.level))
                .or_default()
                .push(p);
        }
        let mut runs = Vec::new();
        for (_, mut parts) in groups {
            parts.sort_by(|a, b| (&a.bounds.first, a.shard).cmp(&(&b.bounds.first, b.shard)));
            runs.push(Run {
                files: parts
                    .into_iter()
                    .map(|p| (root.join(&p.file), None))
                    .collect::<Vec<_>>()
                    .into_iter(),
                reader: None,
                selection: selection.cloned(),
            });
        }
        let mut temporary = Vec::new();
        let mut counter = 0u64;
        while runs.len() > 4 {
            let mut next = Vec::new();
            let mut remaining = runs.into_iter();
            loop {
                let chunk: Vec<_> = remaining.by_ref().take(4).collect();
                if chunk.is_empty() {
                    break;
                }
                let mut merge = Merge::new(chunk, resolve)?;
                let mut files = Vec::new();
                let mut writer: Option<PartWriter> = None;
                let mut current_path = None;
                let mut rows = 0;
                let mut bytes = 0;
                while let Some(row) = merge.next()? {
                    if writer.is_none() {
                        let path = root
                            .join(".staging")
                            .join(format!("merge-{prefix}-{counter}.parquet"));
                        counter += 1;
                        writer = Some(PartWriter::new(&path)?);
                        temporary.push(path.clone());
                        current_path = Some(path);
                    }
                    bytes += row.bytes();
                    rows += 1;
                    writer.as_mut().expect("created writer").push(row)?;
                    if rows >= parquet::PART_ROWS || bytes >= parquet::PART_BYTES {
                        writer.take().expect("active writer").finish()?;
                        files.push((current_path.take().expect("active path"), None));
                        rows = 0;
                        bytes = 0;
                    }
                }
                if let Some(writer) = writer {
                    writer.finish()?;
                    files.push((current_path.expect("active path"), None));
                }
                next.push(Run {
                    files: files.into_iter(),
                    reader: None,
                    selection: None,
                });
            }
            runs = next;
        }
        Ok(Self {
            merge: Merge::new(runs, resolve)?,
            temporary,
        })
    }
}
impl Iterator for ExternalMerge {
    type Item = Result<Record>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.merge.next() {
            Ok(Some(r)) => Some(Ok(r)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
impl Drop for ExternalMerge {
    fn drop(&mut self) {
        for path in &self.temporary {
            let _ = std::fs::remove_file(path);
        }
    }
}
