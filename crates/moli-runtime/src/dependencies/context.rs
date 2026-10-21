//! Bind source versions and every affected calculation before consulting graphs.
use super::{BindingRole, DependencyBinding, DependencyView};
use moli_core::{DependencyKind, DependencyRef, PointId};
use std::collections::{BTreeMap, BTreeSet};

fn reference(point: &PointId) -> Option<DependencyRef> {
    let (id, key) = point.split()?;
    (key == "value").then_some(DependencyRef {
        kind: DependencyKind::Computed,
        id,
    })
}

pub(crate) fn bindings(root: &DependencyView, views: &[DependencyView]) -> Vec<DependencyBinding> {
    let by_ref: BTreeMap<_, _> = views.iter().map(|v| (v.reference.clone(), v)).collect();
    let mut affected = BTreeSet::from([root.reference.clone()]);
    loop {
        let mut next = affected.clone();
        for view in views {
            if [Some(view.versions.draft()), view.versions.current()]
                .into_iter()
                .flatten()
                .flat_map(|v| v.definition.sources())
                .filter_map(reference)
                .any(|source| affected.contains(&source))
            {
                next.insert(view.reference.clone());
            }
        }
        if next == affected {
            break;
        }
        affected = next;
    }
    let mut sources = BTreeSet::new();
    let mut pending: Vec<_> = root
        .versions
        .draft()
        .definition
        .sources()
        .iter()
        .filter_map(reference)
        .collect();
    while let Some(source) = pending.pop() {
        let Some(view) = by_ref.get(&source) else {
            continue;
        };
        if !sources.insert(source) {
            continue;
        }
        if let Some(current) = view.versions.current() {
            pending.extend(current.definition.sources().iter().filter_map(reference));
        }
    }
    let mut result = Vec::new();
    for (refs, role) in [
        (sources, BindingRole::Source),
        (affected, BindingRole::Affected),
    ] {
        for reference in refs {
            if reference == root.reference {
                continue;
            }
            let Some(view) = by_ref.get(&reference) else {
                continue;
            };
            result.push(DependencyBinding {
                reference,
                role,
                current: view.versions.current().map(|v| v.version),
                draft: (role == BindingRole::Affected).then_some(view.versions.draft().version),
            });
        }
    }
    result.sort_by(|a, b| (&a.reference, a.role).cmp(&(&b.reference, b.role)));
    result
}

pub(crate) fn affected<'a>(
    root: &'a DependencyView,
    related: &'a [DependencyBinding],
) -> impl Iterator<Item = &'a DependencyRef> {
    std::iter::once(&root.reference).chain(
        related
            .iter()
            .filter(|r| r.role == BindingRole::Affected)
            .map(|r| &r.reference),
    )
}
