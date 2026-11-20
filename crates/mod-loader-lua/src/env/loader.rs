use mod_loader::Mod;

use crate::{env::{AppFeatures, AppState, util::{add_function, add_function_with_mod}}, lua::{FunctionArgs, LuaError}};

pub fn create(
    lua: &mlua::Lua,
    r#mod: Mod,
) -> mlua::Result<mlua::Table> {
    let app_state = lua.app_data_ref::<AppState>().unwrap();
    let table = lua.create_table()?;

    add_function(lua, &table, "has_mod", lua_has_mod)?;

    table.raw_set("is_client", app_state.is_client())?;
    table.raw_set("is_server", app_state.is_server())?;

    // LoaderFeatures
    let feature_table = lua.create_table()?;

    feature_table.raw_set("patch", app_state.has_feature(AppFeatures::PATCH))?;
    feature_table.raw_set("export", app_state.has_feature(AppFeatures::EXPORT))?;

    // RuntimeFeatures
    let runtime_feature_table = lua.create_table()?;

    runtime_feature_table.raw_set("dll", app_state.has_feature(AppFeatures::RUNTIME_DLL))?;

    feature_table.raw_set("runtime", runtime_feature_table)?;
    table.raw_set("features", feature_table)?;

    // LoaderRuntime
    let runtime_table = lua.create_table()?;

    add_function_with_mod(lua, &runtime_table, "register_dll", &r#mod, lua_register_dll)?;

    table.raw_set("runtime", runtime_table)?;

    Ok(table)
}

fn lua_has_mod(
    lua: &mlua::Lua,
    args: FunctionArgs,
) -> mlua::Result<bool> {
    let app_state = lua.app_data_ref::<AppState>().unwrap();
    let id = args.get::<String>(0)?;

    Ok(app_state.env().mod_registry().contains_key(&id))
}

fn lua_register_dll(
    lua: &mlua::Lua,
    args: FunctionArgs,
    r#mod: &Mod,
) -> mlua::Result<()> {
    let app_state = lua.app_data_ref::<AppState>().unwrap();

    if !app_state.has_feature(AppFeatures::RUNTIME_DLL) {
        return Err(LuaError::generic("register_dll is disabled"));
    }

    let path = args.get::<String>(0)?;
    let mut fs = r#mod.fs();

    if let Some(path) = fs.absolute_path(&path)? {
        app_state.register_dll(path);
    } else {
        // A DLL inside a packed mod would have to be copied out to be loaded. DLLs
        // registered by mods are not loaded in this version, so nothing is copied:
        // the loader writes no file outside the game folder.
        tracing::warn!(
            mod_id = r#mod.info().id,
            path = path,
            "register_dll: DLLs of packed mods are not extracted, ignored",
        );
    }

    Ok(())
}
