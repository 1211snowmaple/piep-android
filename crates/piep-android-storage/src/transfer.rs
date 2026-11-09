use crate::{Error, Fingerprint, NamespaceStore, Result, StorageNamespace, VerifiedFile, protocol};
use std::io::{Read, Write};

/// 通信開始時にネイティブ側で固定する要求の文脈。応答から名前空間を作らない。
#[derive(Clone)]
pub struct TransferExpectation {
    namespace: StorageNamespace,
    target: protocol::TransferTarget,
    manifest_id: Fingerprint,
}
impl TransferExpectation {
    pub fn new(
        namespace: StorageNamespace,
        target: protocol::TransferTarget,
        manifest_id: Fingerprint,
    ) -> Result<Self> {
        if namespace.library() != target.library_id.get() {
            return Err(Error::Integrity("要求と保存領域の棚が一致しません".into()));
        }
        Ok(Self {
            namespace,
            target,
            manifest_id,
        })
    }
    pub fn namespace(&self) -> StorageNamespace {
        self.namespace
    }
    pub fn target(&self) -> &protocol::TransferTarget {
        &self.target
    }
    pub fn manifest_id(&self) -> Fingerprint {
        self.manifest_id
    }
}

/// 要求した資産と元の転送記録を固定する。これは HTTP の認証 receipt ではない。
pub struct PreparedTransfer<'a> {
    store: &'a NamespaceStore,
    manifest: protocol::ValidatedTransfer,
}

impl NamespaceStore {
    /// expected は認証済みの要求・読書 bundle から得る。応答から対象を推測しない。
    /// 接続/同期の epoch の照合と現在参照の切り替えは、呼び手の repository が行う。
    pub fn prepare_transfer(
        &self,
        expected: &TransferExpectation,
        bytes: &[u8],
    ) -> Result<PreparedTransfer<'_>> {
        if self.namespace() != expected.namespace {
            return Err(Error::Integrity(
                "要求と保存領域の名前空間が一致しません".into(),
            ));
        }
        let manifest = protocol::ValidatedTransfer::parse(bytes)?;
        if manifest.manifest().target != expected.target || manifest.id() != expected.manifest_id {
            return Err(Error::Integrity(
                "要求した資産または転送記録と一致しません".into(),
            ));
        }
        Ok(PreparedTransfer {
            store: self,
            manifest,
        })
    }
}

impl PreparedTransfer<'_> {
    pub fn target(&self) -> &protocol::TransferTarget {
        &self.manifest.manifest().target
    }
    pub fn manifest_id(&self) -> Fingerprint {
        self.manifest.id()
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        self.manifest.bytes()
    }
    pub fn namespace(&self) -> StorageNamespace {
        self.store.namespace()
    }

    /// 未取得の順番だけを返す。すでに届いた部分も、この名前空間で再検査する。
    pub fn missing_chunks(&self) -> Result<Vec<usize>> {
        let mut missing = Vec::new();
        for (ordinal, chunk) in self.manifest.file().chunks.iter().enumerate() {
            if !self.store.has_verified_chunk(chunk)? {
                missing.push(ordinal);
            }
        }
        Ok(missing)
    }

    /// 呼び手が自由なハッシュを指定せず、記録に固定した部分へ照合して受信する。
    pub fn receive_chunk(&self, ordinal: usize, source: impl Read) -> Result<()> {
        let chunk = self
            .manifest
            .file()
            .chunks
            .get(ordinal)
            .ok_or_else(|| Error::Integrity("要求した部分の順番が範囲外です".into()))?;
        self.store.receive_chunk(chunk, source)
    }

    /// file 全体がそろい元ハッシュが一致するまで返さない。bundle の完了とは別。
    pub fn verify_file(&self) -> Result<VerifiedTransferFile<'_>> {
        Ok(VerifiedTransferFile {
            target: self.target().clone(),
            manifest_id: self.manifest_id(),
            file: self.store.verify_file(self.manifest.file())?,
        })
    }
}

/// 保存先・要求対象・file の検証結果をまとめる。別の名前空間へ持ち替えない。
pub struct VerifiedTransferFile<'a> {
    target: protocol::TransferTarget,
    manifest_id: Fingerprint,
    file: VerifiedFile<'a>,
}

impl VerifiedTransferFile<'_> {
    pub fn target(&self) -> &protocol::TransferTarget {
        &self.target
    }
    pub fn manifest_id(&self) -> Fingerprint {
        self.manifest_id
    }
    pub fn namespace(&self) -> StorageNamespace {
        self.file.namespace()
    }
    pub fn descriptor(&self) -> &crate::ContentFile {
        self.file.descriptor()
    }
    pub fn export(&self, output: impl Write) -> Result<()> {
        self.file.export(output)
    }
    pub fn read_range(&self, offset: u64, length: u64, output: impl Write) -> Result<()> {
        self.file.read_range(offset, length, output)
    }
}
