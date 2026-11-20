use std::{cell::{Cell, RefCell, RefMut}, collections::{HashMap, hash_map::{self, Entry}}, rc::Rc};

use bitflags::bitflags;
use mod_loader::ModEnvironment;
use once_cell::unsync::OnceCell;
use kfc::{container::{KFCCursor, KFCFile, KFCReader, KFCWriter}, guid::{ContentHash, ResourceId}, reflection::{LookupKey, TypeHandle, TypeIndex, TypeRegistry}, resource::value::Value};

use crate::{RunArgs, alias::{MappedValue, PathBuf}, cache::CacheDiff, env::{Type, game::value::is_dirty_lua_value, value::{convert_lua_to_value, convert_value_to_lua, validate_and_clone_lua_value}}, log::warn, lua::{Held, LuaError, LuaValue}};

pub struct AppState {
    env: ModEnvironment,
    config: AppConfig,

    registered_dlls: RefCell<Vec<PathBuf>>,

    type_registry: Rc<TypeRegistry>,

    ref_file: Rc<KFCFile>,
    reader: RefCell<KFCCursor<KFCReader>>,
    writer: RefCell<Option<KFCWriter<Rc<KFCFile>, Rc<TypeRegistry>>>>,

    // NOTE: the `Vec<u8>` must always be treated as immutable
    new_contents: RefCell<HashMap<ContentHash, Vec<u8>>>,
    resources: RefCell<HashMap<ResourceId, Rc<ResourceInfo>>>,
    types: RefCell<HashMap<TypeIndex, LuaValue>>,

    /// The mod being run, and how many resources it has asked a new id for.
    /// New ids are derived from these, so they are the same in every run.
    running_mod: RefCell<(String, u64)>,

    /// Which mod changed which resource, see [`AppState::record_changes`].
    changes: RefCell<ChangeLog>,
}

/// Which mods changed which resources.
///
/// Only resources that were handed to more than one mod are looked at: when such a
/// resource is handed to the next mod, and once more at the end of the run, it is
/// serialized and compared with what it looked like the time before. A difference
/// is the work of the mod that had it in between.
#[derive(Default)]
struct ChangeLog {
    /// The mods each resource was handed to, in the order they ran.
    handed: HashMap<ResourceId, Vec<String>>,
    /// What a resource looked like when it was last compared. `None` (or no entry)
    /// is the unchanged resource of the game.
    checkpoints: HashMap<ResourceId, Option<u64>>,
    /// The mods that changed a resource, in the order they ran.
    writers: HashMap<ResourceId, Vec<String>>,
}

/// A resource that more than one mod changed.
#[derive(Debug, Clone)]
pub struct SharedResource {
    pub resource_id: ResourceId,
    pub type_name: String,
    /// The mods that changed it, in the order they ran. Later ones saw and may have
    /// replaced what earlier ones did.
    pub mods: Vec<String>,
}

pub struct AppConfig {
    skip_cache: bool,
    skip_version_conflicts: bool,
    is_server: bool,

    feature_flags: AppFeatures,
    export_dir: PathBuf,
}

bitflags! {
    pub struct AppFeatures: u32 {
        const PATCH = 1 << 0;
        const EXPORT = 1 << 1;
        const RUNTIME_DLL = 1 << 2;

        const ALL = !0;
    }
}

impl AppState {

    pub fn new(
        env: ModEnvironment,
        args: RunArgs,
        cache_diff: &CacheDiff,
    ) -> Result<Self, ()> {
        let game_dir = env.game_dir();
        let cache_dir = env.cache_dir();
        let file_name = &args.file_name;

        // prepare configuration

        let options = args.options;
        let export_dir = options.export_dir
            .and_then(|d| {
                d.canonicalize()
                    .map_err(|e| {
                        warn!(
                            error = %e,
                            "Failed to canonicalize export directory, using default instead"
                        );
                    })
                    .ok()
            })
            .and_then(|d| {
                PathBuf::from_path_buf(d)
                    .map_err(|_| {
                        warn!(
                            "Export directory is not valid UTF-8, using default instead"
                        );
                    })
                    .ok()
            })
            .unwrap_or_else(|| env.game_dir().join("export"));

        let skip_cache = options.skip_cache;
        let is_server = options.is_server
            .unwrap_or_else(|| file_name.to_lowercase().contains("server"));

        let mut feature_flags = AppFeatures::empty();

        if options.patch {
            feature_flags |= AppFeatures::PATCH;
        }

        if options.export {
            feature_flags |= AppFeatures::EXPORT;
        }

        if options.runtime {
            feature_flags |= AppFeatures::RUNTIME_DLL;
        }

        let config = AppConfig {
            skip_cache,
            skip_version_conflicts: options.skip_version_conflicts,
            export_dir,

            is_server,

            feature_flags,
        };

        // load and attach to resources

        let (type_registry, is_dirty) = crate::load::load_type_registry(
            game_dir,
            cache_dir,
            file_name,
        )?;
        crate::load::export_lua_definitions(
            cache_dir,
            &type_registry,
            options.skip_cache || is_dirty || cache_diff.build_id_changed(),
        );
        let type_registry = Rc::new(type_registry);

        let kfc_path = game_dir
            .join(file_name)
            .with_extension("kfc");
        let bak_path = crate::load::create_backup(&kfc_path)?;

        let ref_file = crate::load::load_kfc_file(&bak_path)?;
        let reader = crate::load::create_reader(game_dir, file_name)?;
        let writer = crate::load::create_writer(game_dir, file_name, &type_registry, &ref_file)?;

        Ok(Self {
            env,
            config,

            registered_dlls: RefCell::new(Vec::new()),

            type_registry,

            ref_file,
            reader: RefCell::new(reader),
            writer: RefCell::new(Some(writer)),

            new_contents: RefCell::new(HashMap::new()),
            resources: RefCell::new(HashMap::new()),
            types: RefCell::new(HashMap::new()),
            running_mod: RefCell::new((String::new(), 0)),
            changes: RefCell::default(),
        })
    }

    #[inline]
    pub fn env(&self) -> &ModEnvironment {
        &self.env
    }

    #[inline]
    pub fn skip_version_conflicts(&self) -> bool {
        self.config.skip_version_conflicts
    }

    #[inline]
    #[allow(unused)]
    pub fn skip_cache(&self) -> bool {
        self.config.skip_cache
    }

    #[inline]
    pub fn is_server(&self) -> bool {
        self.config.is_server
    }

    #[inline]
    pub fn is_client(&self) -> bool {
        !self.config.is_server
    }

    #[inline]
    pub fn export_dir(&self) -> &PathBuf {
        &self.config.export_dir
    }

    #[inline]
    pub fn has_feature(&self, feature: AppFeatures) -> bool {
        self.config.feature_flags.contains(feature)
    }

    #[inline]
    pub fn register_dll(&self, path: PathBuf) {
        self.registered_dlls.borrow_mut().push(path);
    }

    #[inline]
    pub fn registered_dlls(&self) -> Vec<PathBuf> {
        self.registered_dlls.borrow().clone()
    }

    #[inline]
    pub fn kfc_file(&self) -> &KFCFile {
        &self.ref_file
    }

    #[inline]
    pub fn type_registry(&self) -> &Rc<TypeRegistry> {
        &self.type_registry
    }

    #[inline]
    pub fn reader(&self) -> RefMut<KFCCursor<KFCReader>> {
        self.reader.borrow_mut()
    }

    #[inline]
    pub fn take_writer(
        &self,
    ) -> KFCWriter<Rc<KFCFile>, Rc<TypeRegistry>> {
        self.writer.borrow_mut()
            .take()
            .expect("KFCWriter is no longer available")
    }

    pub fn get_cached_resources(
        &self,
    ) -> Vec<Rc<ResourceInfo>> {
        // in the order of their ids, so that two runs over the same mods write the
        // same bytes (the map's own order changes from run to run)
        let mut resources: Vec<_> = self.resources.borrow()
            .values()
            .cloned()
            .collect();

        resources.sort_by_key(|resource| resource.resource_id);

        resources
    }

    pub fn get_resource_info(
        &self,
        guid: &ResourceId,
        lua: &mlua::Lua,
    ) -> Option<Rc<ResourceInfo>> {
        let info = self.find_resource_info(guid)?;

        self.note_handed(&info, lua);

        Some(info)
    }

    fn find_resource_info(
        &self,
        guid: &ResourceId,
    ) -> Option<Rc<ResourceInfo>> {
        match self.resources.borrow_mut().entry(*guid) {
            Entry::Occupied(entry) => {
                Some(entry.get().clone())
            },
            Entry::Vacant(entry) => {
                if !self.ref_file.resources().contains_key(guid) {
                    return None;
                }

                let info = ResourceInfo {
                    resource_id: *guid,
                    original_value: OnceCell::new(),
                    value: RefCell::default(),
                    is_dirty: Cell::new(false),
                };
                let info = Rc::new(info);

                entry.insert(info.clone());

                Some(info)
            }
        }
    }

    pub fn add_resource(
        &self,
        value: &LuaValue,
        guid: &ResourceId,
        lua: &mlua::Lua,
    ) -> mlua::Result<()> {
        let mut resources = self.resources.borrow_mut();

        // the mod that creates a resource is the first one that has it
        self.changes.borrow_mut().handed.entry(*guid).or_default()
            .push(self.running_mod.borrow().0.clone());

        if resources.contains_key(guid) {
            return Err(LuaError::generic(format!(
                "resource with GUID {guid} already exists"
            )));
        }

        let r#type = TypeHandle::new(
            self.type_registry.clone(),
            self.type_registry.get_by_hash(LookupKey::Qualified(guid.type_hash()))
                .ok_or_else(|| LuaError::type_not_found(guid.type_hash()))?
                .index,
        );
        let value = validate_and_clone_lua_value(
            value,
            &r#type,
            lua,
        ).map_err(LuaError::external)?;

        let info = Rc::new(ResourceInfo {
            resource_id: *guid,
            original_value: OnceCell::new(),
            value: RefCell::new(Some(Held::new(value))),
            is_dirty: Cell::new(true),
        });

        resources.insert(*guid, info);

        Ok(())
    }

    /// Notes that the running mod is handed a resource. If another mod had it
    /// before, this is the moment to see whether that mod changed it.
    fn note_handed(&self, info: &ResourceInfo, lua: &mlua::Lua) {
        let resource_id = info.resource_id;
        let previous = {
            let mut changes = self.changes.borrow_mut();
            let running = self.running_mod.borrow();
            let handed = changes.handed.entry(resource_id).or_default();

            if handed.last().is_some_and(|last| *last == running.0) {
                return;
            }

            let previous = handed.last().cloned();

            handed.push(running.0.clone());

            previous
        };

        if let Some(previous) = previous {
            self.compare_with_checkpoint(info, &previous, lua);
        }
    }

    /// Serializes a resource and compares it with its checkpoint. If it differs,
    /// `mod_id` (the mod that had it since) changed it.
    fn compare_with_checkpoint(&self, info: &ResourceInfo, mod_id: &str, lua: &mlua::Lua) {
        let resource_id = info.resource_id;
        // a value that cannot be applied is reported when the patches are applied
        let now = match info.apply(lua) {
            Ok(Some(value)) => {
                let Some(r#type) = self.type_registry
                    .get_by_hash(LookupKey::Qualified(resource_id.type_hash())) else {
                    return;
                };
                let mut buf = Vec::new();

                if value.write_into(&self.type_registry, r#type, &mut buf).is_err() {
                    return;
                }

                let mut hash: u64 = 0xcbf29ce484222325;

                for byte in &buf {
                    hash ^= u64::from(*byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }

                Some(hash)
            }
            Ok(None) => None,
            Err(_) => return,
        };

        let mut changes = self.changes.borrow_mut();
        let before = changes.checkpoints.insert(resource_id, now).flatten();

        if before != now {
            changes.writers.entry(resource_id).or_default().push(mod_id.to_string());
        }
    }

    /// Called when all mods have run: looks at what the last mod that had each
    /// shared resource did to it.
    pub fn finish_changes(&self, lua: &mlua::Lua) {
        let mut last: Vec<(ResourceId, String)> = self.changes.borrow().handed.iter()
            .filter(|(_, mods)| mods.len() > 1)
            .filter_map(|(resource_id, mods)| Some((*resource_id, mods.last()?.clone())))
            .collect();

        last.sort();

        for (resource_id, mod_id) in last {
            let Some(info) = self.resources.borrow().get(&resource_id).cloned() else {
                continue;
            };

            self.compare_with_checkpoint(&info, &mod_id, lua);
        }
    }

    /// The resources that more than one mod changed, in the order of their ids.
    pub fn shared_resources(&self) -> Vec<SharedResource> {
        let changes = self.changes.borrow();
        let mut shared: Vec<SharedResource> = changes.writers.iter()
            .filter(|(_, mods)| mods.len() > 1)
            .map(|(resource_id, mods)| SharedResource {
                resource_id: *resource_id,
                type_name: self.type_registry
                    .get_by_hash(LookupKey::Qualified(resource_id.type_hash()))
                    .map(|r#type| r#type.qualified_name.to_string())
                    .unwrap_or_default(),
                mods: mods.clone(),
            })
            .collect();

        shared.sort_by_key(|shared| shared.resource_id);

        shared
    }

    /// Tells the state which mod runs now; new resource ids are counted per mod.
    pub fn set_running_mod(&self, mod_id: &str) {
        *self.running_mod.borrow_mut() = (mod_id.to_string(), 0);
    }

    pub fn create_resource(
        &self,
        value: &LuaValue,
        type_index: TypeIndex,
        lua: &mlua::Lua,
    ) -> mlua::Result<ResourceId> {
        let r#type = self.type_registry.get(type_index)
            .ok_or_else(|| LuaError::generic(format!(
                "type with index {type_index} not found"
            )))?;
        let resources = self.resources.borrow_mut();
        let mut guid: ResourceId;

        // The id is derived from the mod and from how many ids it has asked for, not
        // drawn at random: the same mods then give the same game files in every run,
        // and a resource keeps its id from one start of the game to the next.
        let (mod_id, number) = {
            let mut running = self.running_mod.borrow_mut();
            running.1 += 1;
            (running.0.clone(), running.1)
        };
        let mut attempt = 0u32;

        loop {
            guid = ResourceId::new(
                derived_uuid(&mod_id, number, r#type.qualified_hash, attempt).into(),
                r#type.qualified_hash,
                0,
            );

            if !resources.contains_key(&guid) && !self.ref_file.resources().contains_key(&guid) {
                break;
            }

            attempt += 1;
        }

        drop(resources);

        self.add_resource(value, &guid, lua)?;

        Ok(guid)
    }

    pub fn has_content(&self, guid: &ContentHash) -> bool {
        self.new_contents.borrow().contains_key(guid)
            || self.kfc_file().contents().contains_key(guid)
    }

    pub fn get_content(
        &self,
        guid: &ContentHash,
    ) -> mlua::Result<Option<Vec<u8>>> {
        if let Some(value) = self.new_contents.borrow().get(guid) {
            // OPTIMIZE: cloning here can be quite expensive
            Ok(Some(value.clone()))
        } else {
            // OPTIMIZE: consider using a reader for this
            Ok(self.reader.borrow_mut().read_content(guid)?)
        }
    }

    pub fn create_content(
        &self,
        data: &[u8],
    ) -> mlua::Result<ContentHash> {
        let guid = ContentHash::from_data(data);

        if let hash_map::Entry::Vacant(e) = self.new_contents.borrow_mut().entry(guid) {
            self.writer.borrow_mut()
                .as_mut()
                .ok_or_else(|| LuaError::generic(
                    "cannot create content: KFCWriter is no longer available"
                ))?
                .write_content(&guid, data)?;

            // TODO: attach a reader to the writer to avoid cloning
            e.insert(data.to_vec());
        }

        Ok(guid)
    }

    pub fn get_type(
        &self,
        lua: &mlua::Lua,
        type_index: TypeIndex,
    ) -> mlua::Result<Option<LuaValue>> {
        match self.types.borrow_mut().entry(type_index) {
            Entry::Occupied(entry) => Ok(Some(entry.get().clone())),
            Entry::Vacant(entry) => {
                if self.type_registry.get(type_index).is_none() {
                    return Ok(None);
                }

                let value = Type::new(TypeHandle::new(
                    self.type_registry.clone(),
                    type_index,
                ));
                let value = LuaValue::UserData(lua.create_userdata(value)?);

                entry.insert(value.clone());
                Ok(Some(value))
            }
        }
    }

}

pub struct ResourceInfo {
    pub resource_id: ResourceId,

    original_value: OnceCell<Option<MappedValue>>,
    value: RefCell<Option<Held>>,
    is_dirty: Cell<bool>,
}

impl ResourceInfo {

    pub fn get_lua_value(
        &self,
        lua: &mlua::Lua,
    ) -> mlua::Result<LuaValue> {
        if let Some(value) = self.value.borrow().as_ref() {
            return Ok(value.get());
        }

        let value = match self.get_mapped_value(lua)? {
            Some(value) => convert_value_to_lua(value, lua)?,
            None => LuaValue::Nil,
        };

        self.value.replace(Some(Held::new(value.clone())));

        Ok(value)
    }

    pub fn set_lua_value(
        &self,
        lua_value: LuaValue,
        lua: &mlua::Lua,
    ) -> mlua::Result<()> {
        let app_state = lua.app_data_ref::<AppState>().unwrap();
        let type_registry = app_state.type_registry();

        let r#type = type_registry
            .get_by_hash(LookupKey::Qualified(self.resource_id.type_hash()))
            .ok_or_else(|| LuaError::type_not_found(self.resource_id.type_hash()))?;

        let lua_value = validate_and_clone_lua_value(
            &lua_value,
            &TypeHandle::new(
                type_registry.clone(),
                r#type.index,
            ),
            lua,
        ).map_err(LuaError::external)?;

        self.value.replace(Some(Held::new(lua_value)));
        self.is_dirty.replace(true);

        Ok(())
    }

    pub fn apply(
        &self,
        lua: &mlua::Lua,
    ) -> mlua::Result<Option<Value>> {
        let app_state = lua.app_data_ref::<AppState>().unwrap();
        let type_registry = app_state.type_registry();

        let lua_value = match self.value.borrow().as_ref() {
            Some(value) => value.get(),
            None => return Ok(None),
        };
        let lua_value = &lua_value;

        if !self.is_dirty.get() && !is_dirty_lua_value(lua_value)? {
            return Ok(None);
        }

        let r#type = type_registry
            .get_by_hash(LookupKey::Qualified(self.resource_id.type_hash()))
            .ok_or_else(|| LuaError::type_not_found(self.resource_id.type_hash()))?;

        let value = convert_lua_to_value(
            lua_value,
            &TypeHandle::new(
                type_registry.clone(),
                r#type.index,
            ),
        ).map_err(LuaError::external)?;

        Ok(Some(value))
    }

    fn get_mapped_value(
        &self,
        lua: &mlua::Lua,
    ) -> mlua::Result<Option<&MappedValue>> {
        let app_state = lua.app_data_ref::<AppState>().unwrap();

        self.original_value.get_or_try_init(|| {
            match app_state.reader().read_resource(&self.resource_id)? {
                Some(value) => {
                    let type_registry = app_state.type_registry();
                    let r#type = type_registry
                        .get_by_hash(LookupKey::Qualified(self.resource_id.type_hash()))
                        .ok_or_else(|| LuaError::type_not_found(self.resource_id.type_hash()))?;

                    let value = MappedValue::from_bytes(
                        type_registry,
                        r#type,
                        &Rc::from(value.into_boxed_slice()),
                    ).map_err(LuaError::external)?;

                    Ok(Some(value))
                },
                None => Ok(None),
            }
        }).map(|v| v.as_ref())
    }

}

/// The bytes of a version 4 shaped UUID that depends only on its inputs.
fn derived_uuid(mod_id: &str, number: u64, type_hash: u32, attempt: u32) -> [u8; 16] {
    fn fnv(seed: u64, parts: &[&[u8]]) -> u64 {
        let mut hash = seed;

        for part in parts {
            for byte in *part {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
            // a part's end counts, so ("ab", "c") and ("a", "bc") differ
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        // spread the last bytes over the whole value
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xff51afd7ed558ccd);
        hash ^= hash >> 33;

        hash
    }

    let parts: [&[u8]; 4] = [
        mod_id.as_bytes(),
        &number.to_le_bytes(),
        &type_hash.to_le_bytes(),
        &attempt.to_le_bytes(),
    ];

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&fnv(0xcbf29ce484222325, &parts).to_le_bytes());
    bytes[8..].copy_from_slice(&fnv(0x84222325cbf29ce4, &parts).to_le_bytes());

    // mark it as a random-style UUID, like the ids it replaces
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    bytes
}

#[cfg(test)]
mod tests {
    use super::derived_uuid;

    #[test]
    fn new_resource_ids_repeat_and_do_not_collide() {
        assert_eq!(derived_uuid("a_mod", 1, 7, 0), derived_uuid("a_mod", 1, 7, 0));

        let mut seen = std::collections::HashSet::new();

        for mod_id in ["a_mod", "b_mod", "a_mo"] {
            for number in 1..=2000u64 {
                for attempt in 0..2u32 {
                    assert!(seen.insert(derived_uuid(mod_id, number, 7, attempt)));
                }
            }
        }

        assert!(seen.insert(derived_uuid("a_mod", 1, 8, 0)));
    }
}
