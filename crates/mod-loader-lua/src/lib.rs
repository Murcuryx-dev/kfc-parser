use std::path::PathBuf;

use kfc::reflection::LookupKey;
use mod_loader::ModEnvironment;

use crate::{alias::Path, cache::{CacheDiff, FileStateCache}, env::{AppFeatures, AppState}, log::info, runner::LuaModRunner};

mod runner;
mod definition;
mod env;
mod util;
mod log;
mod cache;
mod alias;
mod load;
mod lua;

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// If true, it will ignore the cache and re-apply all mods.
    pub skip_cache: bool,
    /// If None, it will auto-detect based on the files within the game directory.
    pub is_server: Option<bool>,
    /// If true, it will apply patches to the game files.
    pub patch: bool,
    /// If true, it will allow mods to use `io.export` to export files to the export directory.
    pub export: bool,
    /// If true, it will allow mods to use `loader.runtime.register_dll` to register DLLs to be loaded at runtime.
    pub runtime: bool,
    /// If None, it will use the default export directory (`<game_dir>/export`).
    pub export_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RunArgs {
    pub file_name: String,
    /// The features to enable.
    pub options: RunOptions,
}

#[derive(Debug, Clone, Default)]
pub struct RunResult {
    pub dlls: Vec<PathBuf>,
    /// Set when the mods could not be applied. The game can still be started;
    /// the failure says with which data.
    pub failure: Option<RunFailure>,
}

/// Why a run did not apply the mods, and what the game files are left as.
#[derive(Debug, Clone)]
pub struct RunFailure {
    /// The mod that failed, if a mod's own code failed.
    pub mod_id: Option<String>,
    /// What went wrong.
    pub error: String,
    /// The state the game files were left in.
    pub state: GameFileState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameFileState {
    /// Nothing was written: the files are as the last successful run left them.
    Unchanged,
    /// Writing failed part-way and the original index was put back: no mods are applied.
    Unmodded,
    /// Writing failed part-way and the original index could not be put back.
    Broken,
}

impl RunFailure {

    /// A message for the person starting the game.
    pub fn message(&self) -> String {
        let what = match &self.mod_id {
            Some(id) => format!("The mod '{id}' failed:"),
            None => "Applying the mods failed:".to_string(),
        };
        let state = match self.state {
            GameFileState::Unchanged =>
                "Nothing was changed. The game starts with the mods as they were applied the last time it worked. \
                Fix or remove the mod to apply your mods again.",
            GameFileState::Unmodded =>
                "The game files were put back to the original ones. The game starts WITHOUT mods.",
            GameFileState::Broken =>
                "The original game files could not be put back. The game may not start. \
                Verify the game files in Steam.",
        };

        format!("{what}\n\n{}\n\n{state}", self.error)
    }

}

pub fn run(
    env: &ModEnvironment,
    mut args: RunArgs,
) -> anyhow::Result<RunResult> {
    info!("Running lua with options: {:?}", args);

    let kfc_path = env.game_dir().join(format!("{}.kfc", args.file_name));

    // check cache if files have changed

    let cache_diff = if !args.options.skip_cache {
        let current_cache = FileStateCache::read(env.cache_dir());
        let mut new_cache = FileStateCache::default();

        new_cache.track_game_files(env.game_dir());
        new_cache.track_mod_files(env.mods_dir());

        let cache_diff = new_cache.diff(&current_cache);

        if cache_diff.is_none() && args.options.patch {
            args.options.patch = false;

            info!("No changes detected, skipping patching");

            if !args.options.runtime {
                return Ok(RunResult::default());
            }
        }

        cache_diff
    } else {
        info!("Skipping cache check");
        CacheDiff::new_dirty()
    };

    // create a new app state

    let app_state = match AppState::new(
        env.clone(),
        args,
        &cache_diff,
    ) {
        Ok(context) => context,
        Err(_) => anyhow::bail!("Failed to create AppState"),
    };

    // setup and run all mods

    let runner = LuaModRunner::new(app_state)?;

    match runner.setup(env.mod_registry()) {
        Ok(_) => {},
        Err(err) => {
            anyhow::bail!("Failed to setup LuaModRunner: {}", err);
        }
    }

    info!("Running mods...");

    // TODO: move this somewhere else, maybe to the GameContext?

    // A mod that fails stops the run before anything is written: new containers go to
    // temporary files (removed when the writer is dropped) and resources are only
    // written below. The files of the last successful run stay as they are, and the
    // cache is not updated, so the next start tries again.

    if let Err(e) = runner.run() {
        let failure = RunFailure {
            mod_id: Some(e.mod_id),
            error: e.error.to_string(),
            state: GameFileState::Unchanged,
        };

        crate::log::error!(
            mod_id = failure.mod_id,
            error = failure.error,
            "A mod failed, no patches were applied"
        );

        return Ok(RunResult {
            dlls: Vec::new(),
            failure: Some(failure),
        });
    }

    // how many values the loader keeps for the mods (each used to be an mlua reference, of which there can be 1,000,000)
    info!("Lua values kept by the loader: {}", crate::lua::held_slots_used());

    let app_state = runner.lua.app_data_ref::<AppState>().unwrap();

    if app_state.has_feature(AppFeatures::PATCH) {
        info!("Applying patches...");

        // From here the resource data file is written in place, so a failure can leave
        // it out of step with the index. In that case the original index is put back:
        // it only refers to original data, which is never written.

        let applied = (|| -> anyhow::Result<usize> {

        let mut buf = Vec::new();
        let mut writer = app_state.take_writer();
        let resources = app_state.get_cached_resources();
        let mut applied_resources = 0;

        for resource in resources.iter() {
            let resource_id = &resource.resource_id;
            let value = resource.apply(&runner.lua)
                .map_err(|e| anyhow::anyhow!(
                    "Failed to apply resource {resource_id}: {e}"
                ))?;

            if let Some(value) = value {
                let Some(r#type) = app_state.type_registry()
                    .get_by_hash(LookupKey::Qualified(resource_id.type_hash())) else {
                    anyhow::bail!(
                        "Failed to find type with hash {:#010x} for resource {resource_id}",
                        resource_id.type_hash(),
                    );
                };

                buf.clear();

                if let Err(e) = value.write_into(
                    app_state.type_registry(),
                    r#type,
                    &mut buf,
                ) {
                    anyhow::bail!(
                        "Failed to serialize resource {resource_id} of type {}: {e}",
                        r#type.qualified_name,
                    );
                }

                writer.write_resource(resource_id, &buf)?;
                applied_resources += 1;
            }
        }

        writer.finalize()?;

        Ok(applied_resources)
        })();

        match applied {
            Ok(applied_resources) => info!("Applied {} patches", applied_resources),
            Err(e) => {
                let state = match crate::load::restore_backup(&kfc_path) {
                    Ok(()) => GameFileState::Unmodded,
                    Err(()) => GameFileState::Broken,
                };
                let failure = RunFailure {
                    mod_id: None,
                    error: e.to_string(),
                    state,
                };

                crate::log::error!(
                    error = failure.error,
                    state = ?failure.state,
                    "Applying the patches failed"
                );

                // the cache is not updated, so the next start tries again
                return Ok(RunResult {
                    dlls: Vec::new(),
                    failure: Some(failure),
                });
            }
        }
    }

    // create a new cache file

    if app_state.has_feature(AppFeatures::PATCH) {
        let mut new_cache = FileStateCache::default();

        new_cache.track_game_files(env.game_dir());
        new_cache.track_mod_files(env.mods_dir());

        new_cache.write(env.cache_dir());
    } else {
        let mut new_cache = FileStateCache::read(env.cache_dir());

        new_cache.track_game_files(env.game_dir());
        new_cache.write(env.cache_dir());
    }

    Ok(RunResult {
        dlls: app_state.registered_dlls()
            .into_iter()
            .map(|path| path.into_std_path_buf())
            .collect::<Vec<_>>(),
        failure: None,
    })
}

pub fn export_lua_definitions(
    game_dir: impl AsRef<Path>,
    file_name: &str,
    force: bool,
) -> bool {
    let game_dir = game_dir.as_ref();
    let cache_dir = game_dir.join(".cache");
    let (type_registry, is_dirty) = match crate::load::load_type_registry(
        game_dir,
        &cache_dir,
        file_name,
    ) {
        Ok(type_registry) => type_registry,
        Err(_) => return false,
    };

    crate::load::export_lua_definitions(
        &cache_dir,
        &type_registry,
        is_dirty || force,
    );

    true
}

pub fn restore(
    game_dir: impl AsRef<Path>,
    file_name: &str,
) -> bool {
    let kfc_path = game_dir.as_ref()
        .join(format!("{file_name}.kfc"));

    load::restore_backup(&kfc_path).is_ok()
}
