//! 接続・棚・利用者を固定する端末保管。Tauri・WebView・端末 DB から独立する。
//! 接続の認可・epoch・bundle の公開は呼び手の責務で、この層は完成を記録しない。

pub use piep_storage::{ContentFile, Error, FileChunk, Fingerprint, Result};
use piep_storage::{ObjectStore, regular_directory, sync_directory};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// subject は端末ではなく認証された利用者。URL や認証鍵から推測しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageNamespace {
    connection: Uuid,
    library: Uuid,
    subject: Uuid,
}

impl StorageNamespace {
    pub fn new(connection: Uuid, library: Uuid, subject: Uuid) -> Result<Self> {
        if [connection, library, subject].iter().any(Uuid::is_nil) {
            return Err(Error::Integrity("保存領域の識別子が空です".into()));
        }
        Ok(Self {
            connection,
            library,
            subject,
        })
    }
    pub fn connection(self) -> Uuid {
        self.connection
    }
    pub fn library(self) -> Uuid {
        self.library
    }
    pub fn subject(self) -> Uuid {
        self.subject
    }
}

/// パスと裸の共通ストアは外へ渡さない。別の名前空間に探索範囲を広げない。
pub struct NamespaceStore {
    namespace: StorageNamespace,
    objects: ObjectStore,
}

fn checked_child(parent: &Path, name: &str) -> Result<PathBuf> {
    regular_directory(parent)?;
    let child = parent.join(name);
    match fs::create_dir(&child) {
        Ok(()) => sync_directory(parent)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    regular_directory(&child)?;
    Ok(child)
}

impl NamespaceStore {
    /// root はネイティブ側が得た既存のアプリ専用領域だけを渡す。
    /// その祖先を含む管理は OS のアプリ領域に任せ、JS やサーバー指定のパスは渡さない。
    pub fn open(app_data: &Path, namespace: StorageNamespace) -> Result<Self> {
        regular_directory(app_data)?;
        let mut path = app_data.canonicalize()?;
        for name in [
            "library-v2".to_owned(),
            namespace.connection.to_string(),
            namespace.library.to_string(),
            namespace.subject.to_string(),
            "objects".to_owned(),
            "v1".to_owned(),
        ] {
            path = checked_child(&path, &name)?;
        }
        Ok(Self {
            namespace,
            objects: ObjectStore::open(&path)?,
        })
    }

    pub fn namespace(&self) -> StorageNamespace {
        self.namespace
    }

    /// 成功しても chunk だけの検証。この時点で保存済み・読書可能に切り替えない。
    pub fn receive_chunk(&self, expected: &FileChunk, source: impl Read) -> Result<()> {
        self.objects.import_chunk(expected, source)
    }

    /// この名前空間で検証済みかだけを返す。他の利用者のキャッシュを探さない。
    pub fn has_verified_chunk(&self, expected: &FileChunk) -> Result<bool> {
        self.objects.has_verified_chunk(expected)
    }

    /// 元ファイル全体を検査する。所属・接続世代・bundle の確定は別途必要。
    pub fn verify_file(&self, file: &ContentFile) -> Result<VerifiedFile<'_>> {
        self.objects.verify(file)?;
        Ok(VerifiedFile {
            store: self,
            file: file.clone(),
        })
    }
}

/// 検証したストアと descriptor を分離できない参照。認可の receipt ではない。
/// 読み出すたびに保存実体を再検査し、検証後の破損も成功扱いしない。
pub struct VerifiedFile<'a> {
    store: &'a NamespaceStore,
    file: ContentFile,
}

impl VerifiedFile<'_> {
    pub fn namespace(&self) -> StorageNamespace {
        self.store.namespace
    }
    pub fn descriptor(&self) -> &ContentFile {
        &self.file
    }
    pub fn export(&self, output: impl Write) -> Result<()> {
        self.store.objects.export(&self.file, output)
    }
    /// 指定範囲の実体を再検査する。全 chunk の再検査とは区別する。
    pub fn read_range(&self, offset: u64, length: u64, output: impl Write) -> Result<()> {
        self.store
            .objects
            .read_range(&self.file, offset, length, output)
    }
}
