use piep_android_storage::{ContentFile, FileChunk, Fingerprint, NamespaceStore, StorageNamespace};
use piep_storage::ObjectStore;
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
use uuid::Uuid;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("piep-client-storage-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self, namespace: StorageNamespace) -> NamespaceStore {
        NamespaceStore::open(&self.0, namespace).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn namespace() -> StorageNamespace {
    StorageNamespace::new(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()).unwrap()
}
fn wire(source: &ObjectStore, chunk: &FileChunk) -> Vec<u8> {
    let mut bytes = Vec::new();
    source.export_chunk(chunk, &mut bytes).unwrap();
    bytes
}
fn object_root(root: &Path, ns: StorageNamespace) -> PathBuf {
    root.join("library-v2")
        .join(ns.connection().to_string())
        .join(ns.library().to_string())
        .join(ns.subject().to_string())
        .join("objects/v1")
}

#[test]
fn connection_library_and_subject_each_isolate_identical_hashes() {
    let temp = Temp::new();
    let source = ObjectStore::open(&temp.0.join("server")).unwrap();
    let file = source.put(Cursor::new(b"original bytes\0\r\n")).unwrap();
    let ns = namespace();
    let store = temp.store(ns);
    store
        .receive_chunk(&file.chunks[0], Cursor::new(wire(&source, &file.chunks[0])))
        .unwrap();
    let verified = store.verify_file(&file).unwrap();
    assert_eq!(verified.namespace(), ns);
    assert_eq!(verified.descriptor(), &file);
    for other in [
        StorageNamespace::new(Uuid::new_v4(), ns.library(), ns.subject()).unwrap(),
        StorageNamespace::new(ns.connection(), Uuid::new_v4(), ns.subject()).unwrap(),
        StorageNamespace::new(ns.connection(), ns.library(), Uuid::new_v4()).unwrap(),
    ] {
        let other_store = temp.store(other);
        assert!(!other_store.has_verified_chunk(&file.chunks[0]).unwrap());
        assert!(other_store.verify_file(&file).is_err());
        assert!(store.has_verified_chunk(&file.chunks[0]).unwrap());
    }
    let mut bytes = Vec::new();
    verified.export(&mut bytes).unwrap();
    assert_eq!(bytes, b"original bytes\0\r\n");
}

#[test]
fn partial_transfers_survive_restart_but_never_become_verified_files() {
    let temp = Temp::new();
    let source = ObjectStore::open(&temp.0.join("server")).unwrap();
    let input: Vec<_> = (0..8 * 1024 * 1024 + 19).map(|n| (n % 241) as u8).collect();
    let file = source.put(Cursor::new(&input)).unwrap();
    assert_eq!(file.chunks.len(), 3);
    let ns = namespace();
    let store = temp.store(ns);
    store
        .receive_chunk(&file.chunks[0], Cursor::new(wire(&source, &file.chunks[0])))
        .unwrap();
    assert!(store.verify_file(&file).is_err());
    drop(store);
    let resumed = temp.store(ns);
    assert!(resumed.has_verified_chunk(&file.chunks[0]).unwrap());
    assert!(!resumed.has_verified_chunk(&file.chunks[1]).unwrap());
    for chunk in &file.chunks[1..] {
        let bytes = wire(&source, chunk);
        assert!(
            resumed
                .receive_chunk(chunk, Cursor::new(&bytes[..bytes.len() - 1]))
                .is_err()
        );
        assert!(resumed.verify_file(&file).is_err());
        assert!(resumed.has_verified_chunk(&file.chunks[0]).unwrap());
        resumed.receive_chunk(chunk, Cursor::new(bytes)).unwrap();
    }
    let verified = resumed.verify_file(&file).unwrap();
    let mut restored = Vec::new();
    verified.export(&mut restored).unwrap();
    assert_eq!(restored, input);
    let offset = 4 * 1024 * 1024 - 5;
    let mut range = Vec::new();
    verified.read_range(offset, 17, &mut range).unwrap();
    assert_eq!(range, input[offset as usize..offset as usize + 17]);
    assert!(
        verified
            .read_range(file.length, 1, std::io::sink())
            .is_err()
    );
    let wrong = ContentFile {
        id: Fingerprint::of(b"another whole file"),
        ..file
    };
    assert!(resumed.verify_file(&wrong).is_err());
}

#[test]
fn invalid_receive_keeps_existing_verified_bytes_and_removes_temporary_files() {
    let temp = Temp::new();
    let source = ObjectStore::open(&temp.0.join("server")).unwrap();
    let file = source.put(Cursor::new(vec![b'x'; 30_000])).unwrap();
    let ns = namespace();
    let store = temp.store(ns);
    let good = wire(&source, &file.chunks[0]);
    let mut bad = good.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert!(
        store
            .receive_chunk(&file.chunks[0], Cursor::new(&bad))
            .is_err()
    );
    assert!(!store.has_verified_chunk(&file.chunks[0]).unwrap());
    store
        .receive_chunk(&file.chunks[0], Cursor::new(&good))
        .unwrap();
    assert!(
        store
            .receive_chunk(&file.chunks[0], Cursor::new(&bad))
            .is_err()
    );
    store.verify_file(&file).unwrap();
    let files: Vec<_> = fs::read_dir(object_root(&temp.0, ns))
        .unwrap()
        .flat_map(|dir| fs::read_dir(dir.unwrap().path()).unwrap())
        .map(|file| file.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(fs::read(&files[0]).unwrap(), good);
}

#[test]
fn previously_verified_file_is_checked_again_on_read() {
    let temp = Temp::new();
    let source = ObjectStore::open(&temp.0.join("server")).unwrap();
    let file = source.put(Cursor::new(b"content")).unwrap();
    let ns = namespace();
    let store = temp.store(ns);
    store
        .receive_chunk(&file.chunks[0], Cursor::new(wire(&source, &file.chunks[0])))
        .unwrap();
    let verified = store.verify_file(&file).unwrap();
    let hash = file.chunks[0].id.to_string();
    fs::write(
        object_root(&temp.0, ns)
            .join(&hash[..2])
            .join(format!("{hash}.obj")),
        b"broken",
    )
    .unwrap();
    let mut output = Vec::new();
    assert!(verified.export(&mut output).is_err());
    assert!(output.is_empty());
    assert!(verified.read_range(0, 1, std::io::sink()).is_err());
    assert!(store.has_verified_chunk(&file.chunks[0]).is_err());
}

#[test]
fn empty_ids_and_file_in_place_of_directory_are_rejected() {
    let id = Uuid::new_v4();
    assert!(StorageNamespace::new(Uuid::nil(), id, id).is_err());
    assert!(StorageNamespace::new(id, Uuid::nil(), id).is_err());
    assert!(StorageNamespace::new(id, id, Uuid::nil()).is_err());
    let temp = Temp::new();
    let file = temp.0.join("file");
    fs::write(&file, b"kept").unwrap();
    assert!(NamespaceStore::open(&file, namespace()).is_err());
    assert_eq!(fs::read(file).unwrap(), b"kept");
    fs::write(temp.0.join("library-v2"), b"kept").unwrap();
    assert!(NamespaceStore::open(&temp.0, namespace()).is_err());
    assert_eq!(fs::read(temp.0.join("library-v2")).unwrap(), b"kept");
}

#[cfg(unix)]
#[test]
fn symlinked_root_or_namespace_is_not_followed() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let outside = Temp::new();
    let link = temp.0.join("alias");
    symlink(&outside.0, &link).unwrap();
    assert!(NamespaceStore::open(&link, namespace()).is_err());
    symlink(&outside.0, temp.0.join("library-v2")).unwrap();
    assert!(NamespaceStore::open(&temp.0, namespace()).is_err());
    assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
}
