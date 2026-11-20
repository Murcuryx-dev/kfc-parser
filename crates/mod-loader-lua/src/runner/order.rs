use std::collections::{BTreeMap, BTreeSet};

use mod_loader::ModManifest;

/// Why the mods cannot be put in an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderError {
    /// The mod the message is about.
    pub mod_id: String,
    pub message: String,
}

/// The order the mods run in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOrder {
    pub ids: Vec<String>,
    /// Mods left out because of a version conflict, see [`ConflictReport::skipped`].
    pub skipped: Vec<String>,
}

/// A mod that asks for another version of a mod than the one that is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionConflict {
    pub mod_id: String,
    pub mod_name: String,
    pub dependency_id: String,
    /// The version requirement of the mod, for example `^2.0`.
    pub wanted: String,
    pub installed: String,
}

/// The version conflicts among the installed mods.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictReport {
    pub conflicts: Vec<VersionConflict>,
    /// The mods that are left out when the conflicts are skipped: the mods with a
    /// conflict and every mod that needs one of them. Sorted by id.
    pub skipped: Vec<String>,
}

impl ConflictReport {

    pub fn is_empty(&self) -> bool {
        self.conflicts.is_empty()
    }

    /// One line per conflict, and the mods that are left out with them.
    pub fn description(&self) -> String {
        let mut text = String::new();

        for conflict in &self.conflicts {
            text.push_str(&format!(
                "'{}' ({}) needs '{}' in version {}, installed is {}\n",
                conflict.mod_name, conflict.mod_id,
                conflict.dependency_id, conflict.wanted, conflict.installed,
            ));
        }

        let with_conflict: BTreeSet<&str> = self.conflicts.iter()
            .map(|conflict| conflict.mod_id.as_str())
            .collect();
        let dependents: Vec<&str> = self.skipped.iter()
            .map(String::as_str)
            .filter(|id| !with_conflict.contains(id))
            .collect();

        if !dependents.is_empty() {
            text.push_str(&format!(
                "\nMods that need one of these and are left out with them: {}\n",
                dependents.join(", "),
            ));
        }

        text
    }

}

/// Finds the mods that ask for another version of an installed mod than the one
/// that is installed.
pub fn version_conflicts<'a>(
    manifests: impl IntoIterator<Item = &'a ModManifest>,
) -> ConflictReport {
    let mods: BTreeMap<&str, &ModManifest> = manifests.into_iter()
        .map(|manifest| (manifest.id.as_str(), manifest))
        .collect();

    let mut conflicts = Vec::new();

    for (id, manifest) in &mods {
        for dependency in &manifest.dependencies {
            if let Some(found) = mods.get(dependency.id.as_str())
                && !dependency.version.matches(&found.version)
            {
                conflicts.push(VersionConflict {
                    mod_id: id.to_string(),
                    mod_name: manifest.name.clone(),
                    dependency_id: dependency.id.clone(),
                    wanted: dependency.version.to_string(),
                    installed: found.version.to_string(),
                });
            }
        }
    }

    // a mod that needs a mod that is left out is left out as well

    let mut skipped: BTreeSet<&str> = conflicts.iter()
        .map(|conflict| mods[conflict.mod_id.as_str()].id.as_str())
        .collect();

    loop {
        let more: Vec<&str> = mods.iter()
            .filter(|(id, manifest)| {
                !skipped.contains(*id) && manifest.dependencies.iter().any(|dependency| {
                    !dependency.optional.unwrap_or(false)
                        && skipped.contains(dependency.id.as_str())
                })
            })
            .map(|(id, _)| *id)
            .collect();

        if more.is_empty() {
            break;
        }

        skipped.extend(more);
    }

    ConflictReport {
        conflicts,
        skipped: skipped.into_iter().map(str::to_string).collect(),
    }
}

/// Puts the mods in the order they run in.
///
/// A mod runs after every mod it names in `dependencies` and in `run_after`.
/// Mods that do not have to wait for each other run in the order of their ids,
/// so the order is the same on every machine and in every run.
///
/// - a dependency that is not installed is an error, unless it is optional
/// - a dependency in another version than the mod asks for is an error; with
///   `skip_conflicts` the mod is left out instead, with every mod that needs it
/// - a `run_after` entry that is not installed is ignored
/// - mods that wait for each other in a circle are an error
pub fn run_order<'a>(
    manifests: impl IntoIterator<Item = &'a ModManifest>,
    skip_conflicts: bool,
) -> Result<RunOrder, OrderError> {
    let mut mods: BTreeMap<&str, &ModManifest> = manifests.into_iter()
        .map(|manifest| (manifest.id.as_str(), manifest))
        .collect();

    let report = version_conflicts(mods.values().copied());

    if let Some(first) = report.conflicts.first() && !skip_conflicts {
        return Err(OrderError {
            mod_id: first.mod_id.clone(),
            message: format!(
                "A mod needs another version of a mod than the one installed:\n\n{}\n\
                Update the mods so that the versions fit, or remove the mod.",
                report.description(),
            ),
        });
    }

    for id in &report.skipped {
        mods.remove(id.as_str());
    }

    // the installed mods each mod has to wait for
    let mut waits_for: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();

    for (id, manifest) in &mods {
        let waits = waits_for.entry(id).or_default();

        for dependency in &manifest.dependencies {
            match mods.get(dependency.id.as_str()) {
                Some(found) => {
                    if dependency.id != *id {
                        waits.insert(found.id.as_str());
                    }
                }
                // optional, or optional and left out with a version conflict
                None if dependency.optional.unwrap_or(false) => {}
                None => {
                    return Err(OrderError {
                        mod_id: id.to_string(),
                        message: format!(
                            "it needs the mod '{}' ({}), which is not installed",
                            dependency.id, dependency.version,
                        ),
                    });
                }
            }
        }

        for other in &manifest.run_after {
            if let Some(found) = mods.get(other.as_str()) && other != id {
                waits.insert(found.id.as_str());
            }
        }
    }

    // always take the first mod, by id, that waits for nothing that has not run yet

    let mut ids = Vec::with_capacity(mods.len());
    let mut left: BTreeSet<&str> = mods.keys().copied().collect();

    while !left.is_empty() {
        let next = left.iter()
            .copied()
            .find(|id| waits_for[id].iter().all(|other| !left.contains(other)));

        match next {
            Some(id) => {
                left.remove(id);
                ids.push(id.to_string());
            }
            None => {
                // every mod left waits for another one that is left: follow the
                // waiting from the first until a mod comes up a second time
                let mut chain: Vec<&str> = Vec::new();
                let mut at = *left.iter().next().unwrap();

                while !chain.contains(&at) {
                    chain.push(at);
                    at = waits_for[at].iter().copied().find(|other| left.contains(other)).unwrap();
                }

                let start = chain.iter().position(|id| *id == at).unwrap();
                let mut circle: Vec<&str> = chain[start..].to_vec();
                circle.push(at);

                return Err(OrderError {
                    mod_id: at.to_string(),
                    message: format!(
                        "these mods wait for each other in a circle: {}",
                        circle.join(" -> "),
                    ),
                });
            }
        }
    }

    Ok(RunOrder { ids, skipped: report.skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, json: &str) -> ModManifest {
        serde_json::from_str(&format!(
            r#"{{ "id": "{id}", "name": "{id}", "version": "1.2.0" {json} }}"#
        )).unwrap()
    }

    fn order(mods: &[ModManifest]) -> Result<RunOrder, OrderError> {
        run_order(mods.iter(), false)
    }

    #[test]
    fn without_wishes_the_mods_run_by_id() {
        let mods = [manifest("c", ""), manifest("a", ""), manifest("b", "")];

        assert_eq!(order(&mods).unwrap().ids, ["a", "b", "c"]);
    }

    #[test]
    fn a_mod_runs_after_the_ones_it_names() {
        let mods = [
            manifest("a", r#", "run_after": ["c", "not_installed"]"#),
            manifest("b", r#", "dependencies": [{ "id": "d", "version": "^1" }]"#),
            manifest("c", ""),
            manifest("d", ""),
        ];
        let order = order(&mods).unwrap();

        assert_eq!(order.ids, ["c", "a", "d", "b"]);
        assert!(order.skipped.is_empty());
    }

    #[test]
    fn a_missing_dependency_stops_unless_optional() {
        let needed = [manifest("a", r#", "dependencies": [{ "id": "x", "version": "*" }]"#)];
        let error = order(&needed).unwrap_err();

        assert_eq!(error.mod_id, "a");
        assert!(error.message.contains("'x'"));

        let optional = [manifest("a", r#", "dependencies": [{ "id": "x", "version": "*", "optional": true }]"#)];

        assert_eq!(order(&optional).unwrap().ids, ["a"]);
    }

    #[test]
    fn another_version_than_asked_for_is_an_error_that_names_the_mods() {
        let mods = [
            manifest("a", r#", "dependencies": [{ "id": "b", "version": "^2" }]"#),
            manifest("b", ""),
        ];
        let error = order(&mods).unwrap_err();

        assert_eq!(error.mod_id, "a");
        assert!(error.message.contains("'a' (a) needs 'b' in version ^2, installed is 1.2.0"), "{}", error.message);
    }

    #[test]
    fn skipping_leaves_out_the_mod_and_what_needs_it() {
        let mods = [
            manifest("a", r#", "dependencies": [{ "id": "b", "version": "^2" }]"#),
            manifest("b", ""),
            // needs the mod that is left out: left out too, and so is what needs this one
            manifest("c", r#", "dependencies": [{ "id": "a", "version": "*" }]"#),
            manifest("d", r#", "dependencies": [{ "id": "c", "version": "*" }]"#),
            // works without it
            manifest("e", r#", "dependencies": [{ "id": "a", "version": "*", "optional": true }], "run_after": ["a"]"#),
        ];
        let report = version_conflicts(mods.iter());

        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.skipped, ["a", "c", "d"]);
        assert!(report.description().contains("left out with them: c, d"), "{}", report.description());

        let order = run_order(mods.iter(), true).unwrap();

        assert_eq!(order.ids, ["b", "e"]);
        assert_eq!(order.skipped, ["a", "c", "d"]);
    }

    #[test]
    fn a_circle_is_named() {
        let mods = [
            manifest("a", r#", "run_after": ["b"]"#),
            manifest("b", r#", "run_after": ["c"]"#),
            manifest("c", r#", "run_after": ["a"]"#),
            manifest("d", ""),
        ];
        let error = order(&mods).unwrap_err();

        assert_eq!(error.message, "these mods wait for each other in a circle: a -> b -> c -> a");
    }
}
