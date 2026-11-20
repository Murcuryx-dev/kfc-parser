use std::{borrow::Borrow, collections::HashMap, fs::File, io::{BufWriter, Cursor, Seek, SeekFrom, Write}, path::{Path, PathBuf}};

use crate::{Hash32, container::header::ResourceChunkInfo, guid::{ContentHash, ResourceId}, io::{WriteExt, WriteSeekExt}, reflection::TypeRegistry};

use super::{header::{ContentEntry, ContainerInfo, ResourceEntry}, KFCFile, KFCReadError, KFCWriteError, StaticMapBuilder};

const RESOURCE_ALIGNMENT: u64 = 16;
const CONTENT_ALIGNMENT: u64 = 4096;
const RESOURCE_CHUNK_ALIGNMENT: u64 = 4096;
const RESOURCE_CHUNK_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB

pub struct KFCWriter<F, T> {
    type_registry: T,

    path: PathBuf,
    file_name: String,

    /// The index file. It is replaced as the last step of `finalize`.
    kfc_path: PathBuf,

    chunk_writer: Cursor<Vec<u8>>,
    resource_writer: BufWriter<File>,
    uncompressed_offset: u64,
    uncompressed_chunk_offset: u64,
    resource_chunks: Vec<ResourceChunkInfo>,

    container_writers: Vec<ContainerWriter>,

    game_version: String,
    resources: StaticMapBuilder<ResourceId, ResourceEntry>,
    contents: StaticMapBuilder<ContentHash, ContentEntry>,

    incremental_data: Option<IncrementalData<F>>,
    options: KFCWriteOptions,
}

struct IncrementalData<F> {
    reference_file: F,
}

#[derive(Debug)]
pub struct KFCWriteOptions {
    /// The extension to use for kfc files.
    pub kfc_extension: String,
    /// The extension to use for kfc resource files.
    pub resource_extension: String,
    /// The extension to use for dat files.
    pub dat_extension: String,
    /// If true, the writer will overwrite existing data in the .dat files,
    /// instead of causing an error.
    /// **NOTE:** For incremental writes, this will only affect non-default .dat files.
    pub overwrite_containers: bool,
    // TEST: does the game use a i32, u32 or an artifical limit for the dat max size?
    /// The maximum size of a single .dat file before a new one is created.
    pub max_container_size: u32,
    /// Whether to truncate existing .dat files or not.
    /// This is used to remove any leftover data from previous writes.
    /// **NOTE:** For incremental writes, this will only affect non-default .dat files.
    pub truncate_containers: bool,
}

impl Default for KFCWriteOptions {
    fn default() -> Self {
        Self {
            kfc_extension: "kfc".to_string(),
            resource_extension: "kfc_resources".to_string(),
            dat_extension: "dat".to_string(),
            overwrite_containers: false,
            max_container_size: 1024 * 1024 * 1024, // 1 GiB
            truncate_containers: false,
        }
    }
}

impl<F, T> KFCWriter<F, T>
where
    F: Borrow<KFCFile>,
    T: Borrow<TypeRegistry>,
{

    #[inline]
    pub fn new(
        path: impl AsRef<Path>,
        file_name: impl AsRef<str>,
        type_registry: T,
        game_version: impl AsRef<str>,
    ) -> Result<Self, KFCReadError> {
        Self::new_with_options(
            path,
            file_name,
            type_registry,
            game_version,
            KFCWriteOptions::default(),
        )
    }

    pub fn new_with_options(
        path: impl AsRef<Path>,
        file_name: impl AsRef<str>,
        type_registry: T,
        game_version: impl AsRef<str>,
        options: KFCWriteOptions,
    ) -> Result<Self, KFCReadError> {
        let kfc_path = path.as_ref().join(format!(
            "{}.{}",
            file_name.as_ref(),
            options.kfc_extension
        ));
        // fail early if the index cannot be written; it is replaced in `finalize`
        drop(File::options().write(true).read(true).open(&kfc_path)?);

        let resources_path = path.as_ref().join(format!(
            "{}.{}",
            file_name.as_ref(),
            options.resource_extension
        ));
        let resource_file = File::options().write(true).open(&resources_path)?;
        let resource_writer = BufWriter::new(resource_file);

        Ok(Self {
            type_registry,

            path: path.as_ref().to_path_buf(),
            file_name: file_name.as_ref().to_string(),

            kfc_path,

            chunk_writer: Cursor::new(Vec::new()),
            resource_writer,
            uncompressed_offset: 0,
            uncompressed_chunk_offset: 0,
            resource_chunks: Vec::new(),

            container_writers: Vec::new(),

            game_version: game_version.as_ref().to_string(),
            resources: StaticMapBuilder::default(),
            contents: StaticMapBuilder::default(),

            incremental_data: None,
            options,
        })
    }

    #[inline]
    pub fn new_incremental(
        path: impl AsRef<Path>,
        file_name: impl AsRef<str>,
        reference_file: F,
        type_registry: T,
    ) -> Result<Self, KFCReadError> {
        Self::new_incremental_with_options(
            path,
            file_name,
            reference_file,
            type_registry,
            KFCWriteOptions::default(),
        )
    }

    pub fn new_incremental_with_options(
        path: impl AsRef<Path>,
        file_name: impl AsRef<str>,
        reference_file: F,
        type_registry: T,
        options: KFCWriteOptions,
    ) -> Result<Self, KFCReadError> {
        let kfc_path = path.as_ref().join(format!(
            "{}.{}",
            file_name.as_ref(),
            options.kfc_extension
        ));
        let current_file = KFCFile::from_path(&kfc_path, true)?;

        let file = reference_file.borrow();

        let resources = file.resources().as_builder();
        let contents = file.contents().as_builder();

        drop(current_file);

        // fail early if the index cannot be written; it is replaced in `finalize`
        drop(File::options().write(true).read(true).open(&kfc_path)?);

        let resources_path = path.as_ref().join(format!(
            "{}.{}",
            file_name.as_ref(),
            options.resource_extension
        ));
        let resource_file = File::options().write(true).open(&resources_path)?;
        let mut resource_writer = BufWriter::new(resource_file);

        let previous_resource_size = reference_file.borrow()
            .resource_chunks()
            .iter()
            .map(|chunk| chunk.offset + chunk.size)
            .max()
            .unwrap_or(0);

        let previous_uncompressed_size = reference_file.borrow()
            .resource_chunks()
            .iter()
            .map(|chunk| chunk.uncompressed_offset + chunk.uncompressed_size)
            .max()
            .unwrap_or(0);

        resource_writer.seek(SeekFrom::Start(previous_resource_size))?;

        Ok(Self {
            type_registry,

            path: path.as_ref().into(),
            file_name: file_name.as_ref().to_string(),

            kfc_path,
            chunk_writer: Cursor::new(Vec::new()),
            resource_writer,
            uncompressed_offset: previous_uncompressed_size,
            uncompressed_chunk_offset: previous_uncompressed_size,
            resource_chunks: Vec::new(),

            container_writers: Vec::new(),

            game_version: reference_file.borrow().game_version().to_string(),
            resources,
            contents,

            incremental_data: Some(IncrementalData {
                reference_file,
            }),
            options,
        })
    }

    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[inline]
    pub fn type_registry(&self) -> &TypeRegistry {
        self.type_registry.borrow()
    }

    #[inline]
    pub fn reference_file(&self) -> Option<&KFCFile> {
        self.incremental_data.as_ref()
            .map(|data| data.reference_file.borrow())
    }

    pub fn write_resource(
        &mut self,
        guid: &ResourceId,
        mut bytes: &[u8]
    ) -> std::io::Result<()> {
        self.resources.insert(*guid, ResourceEntry {
            offset: self.uncompressed_offset,
            size: bytes.len() as u64
        });

        let previous_offset = self.uncompressed_offset;

        self.uncompressed_offset += bytes.len() as u64;
        self.uncompressed_offset += (RESOURCE_ALIGNMENT - (self.uncompressed_offset % RESOURCE_ALIGNMENT)) % RESOURCE_ALIGNMENT;

        let mut aligned_size = self.uncompressed_offset - previous_offset;

        // write resource data or compress it if needed

        while aligned_size + self.chunk_writer.position() >= RESOURCE_CHUNK_SIZE {
            let available_size = RESOURCE_CHUNK_SIZE - self.chunk_writer.position();
            let split_index = (available_size as usize).min(bytes.len());

            let (chunk_bytes, remaining_bytes) = bytes.split_at(split_index);

            self.chunk_writer.write_all(chunk_bytes)?;
            self.chunk_writer.align(RESOURCE_CHUNK_SIZE as usize)?;

            // compress and submit chunk

            self.submit_resource_data()?;

            // continue with remaining bytes

            bytes = remaining_bytes;
            aligned_size -= available_size;
        }

        if aligned_size > 0 {
            self.chunk_writer.write_all(bytes)?;

            let remaining = aligned_size - bytes.len() as u64;

            if remaining > 0 {
                self.chunk_writer.padding(remaining)?;
            }
        }

        Ok(())
    }

    fn submit_resource_data(&mut self) -> std::io::Result<()> {
        let offset = self.resource_writer.stream_position()?;
        let chunk_data = self.chunk_writer.get_ref();

        zstd::stream::copy_encode(
            &mut &chunk_data[..],
            &mut self.resource_writer,
            0
        )?;

        let compressed_size = self.resource_writer.stream_position()? - offset;

        self.resource_writer.align(RESOURCE_CHUNK_ALIGNMENT as usize)?;

        let size = self.resource_writer.stream_position()? - offset;
        let uncompressed_offset = self.uncompressed_chunk_offset;
        let uncompressed_size = chunk_data.len() as u64;

        self.resource_chunks.push(ResourceChunkInfo {
            offset,
            size,
            compressed_size,
            uncompressed_offset,
            uncompressed_size,
        });

        self.uncompressed_chunk_offset += uncompressed_size;

        // reset chunk writer

        self.chunk_writer.set_position(0);
        self.chunk_writer.get_mut().clear();

        Ok(())
    }

    pub fn write_content(
        &mut self,
        guid: &ContentHash,
        data: &[u8],
    ) -> std::io::Result<()> {
        if guid.size() != data.len() as u32 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Content size mismatch"));
        }

        if self.contents.contains_key(guid) {
            // if the content already exists, we don't need to write it again since they are unique
            return Ok(());
        }

        let container_writer = self.get_container_writer()?;
        let index = container_writer.index;

        let writer = container_writer.aquire()?;
        let offset = writer.stream_position()?;

        writer.write_all(data)?;
        writer.align(CONTENT_ALIGNMENT as usize)?;

        self.contents.insert(*guid, ContentEntry::new(offset, 0, index));

        Ok(())
    }

    pub fn finalize(mut self) -> Result<(), KFCWriteError> {
        self.submit_resource_data()?;

        // prepare container infos

        let mut containers = self.incremental_data.as_ref()
            .map(|data| data.reference_file.borrow().containers().to_vec())
            .unwrap_or_default();

        containers.reserve_exact(self.container_writers.len());

        self.container_writers.sort_by_key(|w| w.index);

        for container_writer in &mut self.container_writers {
            container_writer.flush()?;

            containers.push(ContainerInfo {
                size: container_writer.position()?,
                count: container_writer.count,
            });
        }

        // make sure to have a power of two container files

        let required_container_count = containers.len().next_power_of_two();
        let mut padding_paths = Vec::new();

        while containers.len() < required_container_count {
            let path = self.dat_path(containers.len());

            // Incremental padding follows all reference and newly written containers.
            // Clear stale mod data so these files match their zero-size index entries,
            // even when callers leave truncate_containers at its default value.
            if !path.exists() || self.options.truncate_containers || self.incremental_data.is_some() {
                padding_paths.push(path);
            }

            containers.push(ContainerInfo {
                size: 0,
                count: 0,
            });
        }

        // prepare resource chunks

        let mut chunks = self.incremental_data.as_ref()
            .map(|data| data.reference_file.borrow().resource_chunks().to_vec())
            .unwrap_or_default();

        chunks.extend(self.resource_chunks.iter().cloned());

        // header construction

        let kfc_temp_path = temp_path(&self.kfc_path);
        let mut file_writer = BufWriter::new(File::create(&kfc_temp_path)?);
        let mut file = KFCFile::default();

        file.set_game_version(self.game_version);
        file.set_resources(
            self.resources.build(),
            self.type_registry.borrow(),
            self.incremental_data.as_ref()
                .map(|data| data.reference_file.borrow().resource_bundles())
        )?;
        file.set_contents(self.contents.build());
        file.set_containers(containers);
        file.set_resource_chunks(chunks);

        file.write(&mut file_writer)?;

        // The index is written beside the old one and replaces it in one step, so
        // an interrupted write cannot leave a half-written index behind.

        file_writer.flush()?;
        file_writer.get_ref().sync_all()?;
        drop(file_writer);

        // Everything is written. Up to here nothing the current index refers to has
        // been touched. Now the new containers take their final names, the padding
        // containers are emptied, and the new index replaces the old one last.

        for container_writer in std::mem::take(&mut self.container_writers) {
            container_writer.commit()?;
        }

        for path in padding_paths {
            File::create(path)?;
        }

        std::fs::rename(&kfc_temp_path, &self.kfc_path)?;

        Ok(())
    }

    pub fn dat_path(&self, index: usize) -> PathBuf {
        // Format: FILE_NAME_{INDEX}.dat where INDEX is 3 digits with leading zeros
        let name = format!(
            "{}_{:03}.{}",
            self.file_name,
            index,
            self.options.dat_extension
        );
        self.path.join(name)
    }

    fn get_container_writer(&mut self) -> std::io::Result<&mut ContainerWriter> {
        if !self.container_writers.is_empty() {
            let writer = self.container_writers.last_mut().unwrap();
            let pos = writer.position()?;

            if pos < self.options.max_container_size as u64 {
                return Ok(self.container_writers.last_mut().unwrap());
            }
        }

        // create a new container writer

        let base_index = self.incremental_data.as_ref()
            .map(|data| data.reference_file.borrow().containers().len())
            .unwrap_or(0);
        let next_index = base_index + self.container_writers.len();
        let path = self.dat_path(next_index);

        if !self.options.overwrite_containers && path.exists() {
            // make sure we don't accidentally overwrite an existing file
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Data file already exists: {}", path.display())
            ));
        }

        let temp_path = temp_path(&path);
        let writer = BufWriter::new(File::create(&temp_path)?);
        let writer = ContainerWriter::new(next_index, writer, path, temp_path);

        self.container_writers.push(writer);

        Ok(self.container_writers.last_mut().unwrap())
    }

}

/// The name a file is written under until it is complete.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

/// Writes one new container. The data goes to a temporary file beside the final
/// one, which `commit` moves into place. A writer that is dropped without a commit
/// removes its temporary file, so an aborted run leaves the existing containers,
/// and with them the current index, intact.
struct ContainerWriter {
    index: usize,
    count: usize,
    /// `None` once the file is closed.
    writer: Option<BufWriter<File>>,
    path: PathBuf,
    temp_path: PathBuf,
    committed: bool,
}

impl Drop for ContainerWriter {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

impl ContainerWriter {

    #[inline]
    fn new(index: usize, writer: BufWriter<File>, path: PathBuf, temp_path: PathBuf) -> Self {
        Self {
            index,
            count: 0,
            writer: Some(writer),
            path,
            temp_path,
            committed: false,
        }
    }

    /// Moves the finished container to its final name, replacing an older one.
    fn commit(mut self) -> std::io::Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
            writer.get_ref().sync_all()?;
        }

        // the file must be closed before it can be renamed on Windows
        drop(self.writer.take());

        std::fs::rename(&self.temp_path, &self.path)?;
        self.committed = true;

        Ok(())
    }

    #[inline]
    fn aquire(&mut self) -> std::io::Result<&mut BufWriter<File>> {
        self.count += 1;
        Ok(self.writer.as_mut().expect("container writer is closed"))
    }

    #[inline]
    fn position(&mut self) -> std::io::Result<u64> {
        self.writer.as_mut().expect("container writer is closed").stream_position()
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.as_mut().expect("container writer is closed").flush()
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const NAME: &str = "incremental-padding-test";

    fn reference(directory: &Path) -> KFCFile {
        let mut reference = KFCFile::default();
        reference.set_game_version("padding-regression".into());
        let mut containers = Vec::new();
        for index in 0..3 {
            let bytes = format!("original container {index}").into_bytes();
            fs::write(directory.join(format!("{NAME}_{index:03}.dat")), &bytes).unwrap();
            containers.push(ContainerInfo { size: bytes.len() as u64, count: 0 });
        }
        reference.set_containers(containers);
        reference.write(&mut File::create(directory.join(format!("{NAME}.kfc"))).unwrap()).unwrap();
        File::create(directory.join(format!("{NAME}.kfc_resources"))).unwrap();
        reference
    }

    fn patch(directory: &Path, reference: &KFCFile, count: usize) -> Result<(), KFCWriteError> {
        let mut writer = KFCWriter::new_incremental_with_options(
            directory, NAME, reference, TypeRegistry::default(),
            KFCWriteOptions { overwrite_containers: true, max_container_size: 4096, ..Default::default() },
        ).unwrap();
        for index in 0..count {
            let bytes = vec![index as u8 + 1; 4096];
            writer.write_content(&ContentHash::from_data(&bytes), &bytes).unwrap();
        }
        writer.finalize()
    }

    #[test]
    fn incremental_padding_clears_stale_mod_content_without_touching_originals() {
        for count in [1, 3] {
            let directory = tempfile::tempdir().unwrap();
            let reference = reference(directory.path());
            let originals: Vec<_> = (0..3).map(|index|
                fs::read(directory.path().join(format!("{NAME}_{index:03}.dat"))).unwrap()).collect();
            for _ in 0..2 {
                for count in [count, 0] {
                    patch(directory.path(), &reference, count).unwrap();
                    let actual = KFCFile::from_path(directory.path().join(format!("{NAME}.kfc")), false).unwrap();
                    for (index, container) in actual.containers().iter().enumerate() {
                        assert_eq!(fs::metadata(directory.path().join(format!("{NAME}_{index:03}.dat"))).unwrap().len(),
                                   container.size, "container {index} after {count} writes");
                    }
                    for (index, expected) in originals.iter().enumerate() {
                        assert_eq!(&fs::read(directory.path().join(format!("{NAME}_{index:03}.dat"))).unwrap(), expected);
                    }
                }
            }
        }
    }

    fn snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(directory).unwrap()
            .map(|entry| entry.unwrap().path())
            .map(|path| (path.file_name().unwrap().to_string_lossy().into_owned(), fs::read(&path).unwrap()))
            .collect();
        files.sort();
        files
    }

    #[test]
    fn aborted_run_leaves_index_and_containers_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let reference = reference(directory.path());

        patch(directory.path(), &reference, 3).unwrap();
        let before = snapshot(directory.path());
        assert!(before.iter().all(|(name, _)| !name.ends_with(".tmp")), "a finished run leaves no temporary files");

        // a second run writes other content and is dropped before `finalize`, as when a mod fails
        let mut writer = KFCWriter::new_incremental_with_options(
            directory.path(), NAME, &reference, TypeRegistry::default(),
            KFCWriteOptions { overwrite_containers: true, max_container_size: 4096, ..Default::default() },
        ).unwrap();
        for index in 0..2 {
            let bytes = vec![index as u8 + 100; 4096];
            writer.write_content(&ContentHash::from_data(&bytes), &bytes).unwrap();
        }
        assert!(snapshot(directory.path()).iter().any(|(name, _)| name.ends_with(".tmp")), "new containers are written beside the old ones");
        drop(writer);

        assert_eq!(snapshot(directory.path()), before);
    }

    #[test]
    fn incremental_padding_propagates_failure_to_truncate_stale_slot() {
        let directory = tempfile::tempdir().unwrap();
        let reference = reference(directory.path());
        fs::create_dir(directory.path().join(format!("{NAME}_003.dat"))).unwrap();
        assert!(patch(directory.path(), &reference, 0).is_err());
    }
}
