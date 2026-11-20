use std::path::PathBuf;

use camino::{Utf8Path, Utf8PathBuf};
use mod_loader::{lua::{self, RunArgs, RunOptions}, runtime, Config, ModEnvironment};

use crate::{log::error, logging};

pub fn init(config: Config) {
    logging::setup();

    let current_dir = std::env::current_dir()
        .expect("Failed to get current directory");
    let current_dir = Utf8PathBuf::from_path_buf(current_dir)
        .expect("Current directory path is not valid UTF-8");

    let env = match ModEnvironment::load(&current_dir) {
        Ok(env) => env,
        Err(e) => {
            if let Some(error) = e.error {
                error!(
                    error = %error,
                    "Error loading mod environment"
                );
                // The mods folder could not be read. Nothing is patched; the game starts
                // with the files as the last successful run left them.
                crate::show_message(
                    "Enshrouded Mod Loader",
                    &format!("The mods could not be loaded:\n\n{error}\n\nNothing was changed. The game starts with the mods as they were applied the last time it worked."),
                );
                return;
            }

            let mut report = String::from("Errors loading some mods:");

            for mod_error in e.mods {
                if let Some(id) = &mod_error.id {
                    report += &format!(
                        "\n  In mod '{}' at {}: {}",
                        id,
                        mod_error.path,
                        mod_error.error
                    );
                } else {
                    report += &format!(
                        "\n  In mod at {}: {}",
                        mod_error.path,
                        mod_error.error
                    );
                }
            }

            error!(
                report = %report,
                "Error loading mod environment"
            );
            // A mod that cannot be read stops the others from being applied this
            // start, but not the game.
            crate::show_message(
                "Enshrouded Mod Loader",
                &format!("{report}\n\nNothing was changed. The game starts with the mods as they were applied the last time it worked. Fix or remove the mods named above to apply your mods again."),
            );
            return;
        }
    };

    let result = lua::run(
        &env,
        RunArgs {
            file_name: get_file_name(&current_dir),
            options: RunOptions {
                patch: true,
                export: config.use_export_flag,
                export_dir: config.export_directory.map(PathBuf::from),
                runtime: true,
                ..Default::default()
            },
        },
    );

    let result = match result {
        Ok(result) => result,
        Err(e) => {
            error!(
                error = %e,
                "Error running mod loader"
            );
            // The game files were not touched by this (a failure after that point is
            // reported through `result.failure`): let the game start.
            crate::show_message(
                "Enshrouded Mod Loader",
                &format!("The mod loader could not run:\n\n{e}\n\nThe game starts with the mods as they were applied the last time it worked."),
            );
            return;
        }
    };

    // The mods were not applied, but the game files are in a known state: tell the
    // player what happened and let the game start.
    if let Some(failure) = &result.failure {
        crate::show_message("Enshrouded Mod Loader", &failure.message());
    }

    runtime::loader_attach(
        &env,
        runtime::RuntimeOptions {
            dlls: result.dlls,
        },
    ).expect("Failed to attach runtime loader");
}

pub fn deinit() {
    runtime::loader_detach()
        .expect("Failed to detach runtime loader");
}

fn get_file_name(
    game_dir: &Utf8Path,
) -> String {
    if game_dir.join("enshrouded.kfc").exists() || game_dir.join("enshrouded.exe").exists() {
        "enshrouded".into()
    } else if game_dir.join("enshrouded_server.kfc").exists() || game_dir.join("enshrouded_server.exe").exists() {
        "enshrouded_server".into()
    } else {
        // try to find file with .kfc or .exe extension
        for file in std::fs::read_dir(game_dir).into_iter().flatten().flatten() {
            if let Some(ext) = file.path().extension() {
                if ext == "kfc" || ext == "exe" {
                    let file = file.path();
                    let file_name = file.file_stem()
                        .and_then(|s| s.to_str());

                    if let Some(file_name) = file_name {
                        return file_name.into();
                    }
                }
            }
        }

        error!(
            game_dir = %game_dir,
            "Could not find required game files"
        );
        panic!("Could not find required game files in {game_dir}");
    }
}
