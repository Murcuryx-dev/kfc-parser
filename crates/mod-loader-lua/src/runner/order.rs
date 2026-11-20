use std::collections::{BTreeMap, BTreeSet};

use mod_loader::ModManifest;

/// Why the mods cannot be put in an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderError {
    /// The mod the message is about.
    pub mod_id: String,
    pub message: String,
}

/// The order the mods run in, and what to tell the user about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOrder {
    pub ids: Vec<String>,
    pub warnings: Vec<String>,
}

/// Puts the mods in the order they run in.
///
/// A mod runs after every mod it names in `dependencies` and in `run_after`.
/// Mods that do not have to wait for each other run in the order of their ids,
/// so the order is the same on every machine and in every run.
///
/// - a dependency that is not installed is an error, unless it is optional
/// - a dependency with a version the mod did not ask for is a warning
/// - a `run_after` entry that is not installed is ignored
/// - mods that wait for each other in a circle are an error
pub fn run_order<'a>(
    manifests: impl IntoIterator<Item = &'a ModManifest>,
) -> Result<RunOrder, OrderError> {
    let mods: BTreeMap<&str, &ModManifest> = manifests.into_iter()
        .map(|manifest| (manifest.id.as_str(), manifest))
        .collect();

    let mut warnings = Vec::new();
    // the installed mods each mod has to wait for
    let mut waits_for: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();

    for (id, manifest) in &mods {
        let waits = waits_for.entry(id).or_default();

        for dependency in &manifest.dependencies {
            match mods.get(dependency.id.as_str()) {
                Some(found) => {
                    if !dependency.version.matches(&found.version) {
                        warnings.push(format!(
                            "'{id}' asks for '{}' {}, installed is {}",
                            dependency.id, dependency.version, found.version,
                        ));
                    }

                    if dependency.id != *id {
                        waits.insert(found.id.as_str());
                    }
                }
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

    Ok(RunOrder { ids, warnings })
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
        run_order(mods.iter())
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
        assert!(order.warnings.is_empty());
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
    fn another_version_than_asked_for_is_a_warning() {
        let mods = [
            manifest("a", r#", "dependencies": [{ "id": "b", "version": "^2" }]"#),
            manifest("b", ""),
        ];
        let order = order(&mods).unwrap();

        assert_eq!(order.ids, ["b", "a"]);
        assert_eq!(order.warnings.len(), 1);
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
