use piep_android_storage::{
    Fingerprint, NamespaceStore, StorageNamespace, TransferExpectation,
    protocol::{self, Id, ResourceReference, ResourceScope, TransferManifest, TransferTarget},
};
use piep_storage::{ContentFile, ObjectStore};
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
};
use uuid::Uuid;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("piep-request-transfer-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn source(&self) -> ObjectStore {
        ObjectStore::open(&self.0.join("source")).unwrap()
    }
    fn receiver(&self, ns: StorageNamespace) -> NamespaceStore {
        NamespaceStore::open(&self.0, ns).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn id() -> Id {
    Id::new(Uuid::new_v4()).unwrap()
}
fn target() -> TransferTarget {
    TransferTarget {
        library_id: id(),
        entity_id: id(),
        scope: ResourceScope::Snapshot(id()),
        resource: ResourceReference::Media {
            slot: "cover".into(),
        },
    }
}
fn namespace(target: &TransferTarget) -> StorageNamespace {
    StorageNamespace::new(Uuid::new_v4(), target.library_id.get(), Uuid::new_v4()).unwrap()
}
fn descriptor(target: &TransferTarget, file: &ContentFile) -> Vec<u8> {
    TransferManifest::new(target.clone(), file)
        .unwrap()
        .encode()
        .unwrap()
}
fn expected(ns: StorageNamespace, target: &TransferTarget, bytes: &[u8]) -> TransferExpectation {
    TransferExpectation::new(ns, target.clone(), Fingerprint::of(bytes)).unwrap()
}
fn encoded(source: &ObjectStore, file: &ContentFile, index: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    source
        .export_chunk(&file.chunks[index], &mut bytes)
        .unwrap();
    bytes
}

#[test]
fn requested_namespace_target_and_manifest_identity_are_all_required() {
    let temp = Temp::new();
    let source = temp.source();
    let file = source.put(Cursor::new(b"original")).unwrap();
    let target = target();
    let ns = namespace(&target);
    let receiver = temp.receiver(ns);
    let bytes = descriptor(&target, &file);
    let expectation = expected(ns, &target, &bytes);
    assert!(receiver.prepare_transfer(&expectation, &bytes).is_ok());
    for mut wrong in [
        target.clone(),
        target.clone(),
        target.clone(),
        target.clone(),
        target.clone(),
    ]
    .into_iter()
    .enumerate()
    {
        match wrong.0 {
            0 => wrong.1.entity_id = id(),
            1 => wrong.1.scope = ResourceScope::Snapshot(id()),
            2 => {
                wrong.1.scope = match target.scope {
                    ResourceScope::Snapshot(id) => ResourceScope::Edit(id),
                    _ => unreachable!(),
                }
            }
            3 => wrong.1.resource = ResourceReference::Body,
            4 => {
                wrong.1.resource = ResourceReference::Media {
                    slot: "another".into(),
                }
            }
            _ => unreachable!(),
        }
        let wrong_bytes = descriptor(&wrong.1, &file);
        // 応答の hash が既知でも、要求対象を応答に合わせて持ち替えない。
        let pinned_hash = expected(ns, &target, &wrong_bytes);
        assert!(
            receiver
                .prepare_transfer(&pinned_hash, &wrong_bytes)
                .is_err()
        );
    }
    let mut reserialized = bytes.clone();
    reserialized.push(b'\n');
    assert!(
        receiver
            .prepare_transfer(&expectation, &reserialized)
            .is_err()
    );
    for other in [
        StorageNamespace::new(Uuid::new_v4(), ns.library(), ns.subject()).unwrap(),
        StorageNamespace::new(ns.connection(), ns.library(), Uuid::new_v4()).unwrap(),
        StorageNamespace::new(ns.connection(), Uuid::new_v4(), ns.subject()).unwrap(),
    ] {
        assert!(
            temp.receiver(other)
                .prepare_transfer(&expectation, &bytes)
                .is_err()
        );
    }
    assert!(!receiver.has_verified_chunk(&file.chunks[0]).unwrap());
    assert!(
        TransferExpectation::new(
            StorageNamespace::new(ns.connection(), Uuid::new_v4(), ns.subject()).unwrap(),
            target,
            expectation.manifest_id()
        )
        .is_err()
    );
}

#[test]
fn partial_transfer_reopens_in_the_same_namespace_and_keeps_the_pinned_target() {
    let temp = Temp::new();
    let source = temp.source();
    let original: Vec<u8> = (0..8 * 1024 * 1024 + 27).map(|n| (n % 251) as u8).collect();
    let file = source.put(Cursor::new(&original)).unwrap();
    let target = target();
    let ns = namespace(&target);
    let receiver = temp.receiver(ns);
    let raw = descriptor(&target, &file);
    let expected = expected(ns, &target, &raw);
    let transfer = receiver.prepare_transfer(&expected, &raw).unwrap();
    assert_eq!(transfer.missing_chunks().unwrap(), vec![0, 1, 2]);
    assert!(
        transfer
            .receive_chunk(0, Cursor::new(encoded(&source, &file, 1)))
            .is_err()
    );
    transfer
        .receive_chunk(0, Cursor::new(encoded(&source, &file, 0)))
        .unwrap();
    assert!(transfer.verify_file().is_err());
    drop(transfer);
    drop(receiver);
    let receiver = temp.receiver(ns);
    let transfer = receiver.prepare_transfer(&expected, &raw).unwrap();
    assert_eq!(transfer.missing_chunks().unwrap(), vec![1, 2]);
    for ordinal in transfer.missing_chunks().unwrap() {
        let mut broken = encoded(&source, &file, ordinal);
        broken.pop();
        assert!(
            transfer
                .receive_chunk(ordinal, Cursor::new(broken))
                .is_err()
        );
        assert!(transfer.verify_file().is_err());
        transfer
            .receive_chunk(ordinal, Cursor::new(encoded(&source, &file, ordinal)))
            .unwrap();
    }
    assert!(transfer.missing_chunks().unwrap().is_empty());
    let verified = transfer.verify_file().unwrap();
    assert_eq!(verified.target(), &target);
    assert_eq!(verified.namespace(), ns);
    assert_eq!(verified.manifest_id(), expected.manifest_id());
    assert_eq!(transfer.manifest_bytes(), raw);
    let mut restored = Vec::new();
    verified.export(&mut restored).unwrap();
    assert_eq!(restored, original);
    let mut range = Vec::new();
    verified
        .read_range(4 * 1024 * 1024 - 2, 7, &mut range)
        .unwrap();
    assert_eq!(range, original[4 * 1024 * 1024 - 2..4 * 1024 * 1024 + 5]);
}

#[test]
fn all_chunks_received_is_not_equivalent_to_whole_file_verified() {
    let temp = Temp::new();
    let source = temp.source();
    let file = source.put(Cursor::new(vec![b'x'; 30_000])).unwrap();
    let target = target();
    let ns = namespace(&target);
    let receiver = temp.receiver(ns);
    let mut manifest = TransferManifest::new(target.clone(), &file).unwrap();
    manifest.file.id = protocol::Hash::new(Fingerprint::of(b"wrong full file"));
    let bytes = manifest.encode().unwrap();
    let transfer = receiver
        .prepare_transfer(&expected(ns, &target, &bytes), &bytes)
        .unwrap();
    transfer
        .receive_chunk(0, Cursor::new(encoded(&source, &file, 0)))
        .unwrap();
    assert!(transfer.missing_chunks().unwrap().is_empty());
    assert!(transfer.verify_file().is_err());
}

#[test]
fn invalid_ordinal_never_reads_an_unrelated_stream_and_invalid_manifest_never_receives() {
    struct MustNotRead;
    impl Read for MustNotRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("unexpected read")
        }
    }
    let temp = Temp::new();
    let source = temp.source();
    let file = source.put(Cursor::new(b"data")).unwrap();
    let target = target();
    let ns = namespace(&target);
    let receiver = temp.receiver(ns);
    let raw = descriptor(&target, &file);
    let expected = expected(ns, &target, &raw);
    let transfer = receiver.prepare_transfer(&expected, &raw).unwrap();
    assert!(transfer.receive_chunk(1, MustNotRead).is_err());
    assert!(
        receiver
            .prepare_transfer(&expected, b"{\"format\":\"unknown\"}")
            .is_err()
    );
    assert!(!receiver.has_verified_chunk(&file.chunks[0]).unwrap());
}

#[test]
fn a_second_subject_cannot_reuse_another_subjects_received_chunks() {
    let temp = Temp::new();
    let source = temp.source();
    let file = source.put(Cursor::new(b"same known content")).unwrap();
    let target = target();
    let ns = namespace(&target);
    let first = temp.receiver(ns);
    let raw = descriptor(&target, &file);
    let old = expected(ns, &target, &raw);
    let transfer = first.prepare_transfer(&old, &raw).unwrap();
    transfer
        .receive_chunk(0, Cursor::new(encoded(&source, &file, 0)))
        .unwrap();
    transfer.verify_file().unwrap();
    let another = StorageNamespace::new(ns.connection(), ns.library(), Uuid::new_v4()).unwrap();
    let second = temp.receiver(another);
    assert!(second.prepare_transfer(&old, &raw).is_err());
    let own = expected(another, &target, &raw);
    let second_transfer = second.prepare_transfer(&own, &raw).unwrap();
    assert_eq!(second_transfer.missing_chunks().unwrap(), vec![0]);
    assert!(second_transfer.verify_file().is_err());
}
