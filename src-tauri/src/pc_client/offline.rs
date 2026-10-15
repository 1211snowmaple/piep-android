//! 利用者が選んだ作品だけを、検証後に版単位で切り替えて保存する。

use super::*;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use uuid::Uuid;

const MAX_PAGES: usize = 10_000;
const MAX_ASSETS: usize = 10_000;
const MAX_SNAPSHOT_BYTES: usize = 500 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_COLLECTION_COVERS: usize = 64;
const MAX_COLLECTION_COVER_BYTES: usize = 16 * 1024 * 1024;

pub struct OfflineState {
    root: PathBuf,
    save_lock: Mutex<()>,
    lock: Mutex<()>,
}

impl OfflineState {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            save_lock: Mutex::new(()),
            lock: Mutex::new(()),
        }
    }

    fn work_dir(&self, work_id: i64) -> Result<PathBuf, ClientError> {
        if work_id <= 0 {
            return Err(ClientError::remote());
        }
        Ok(self.root.join(format!("work-{work_id}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineWork {
    pub work: MobileWork,
    #[serde(default)]
    pub people: Vec<MobilePerson>,
    #[serde(default)]
    pub series: Vec<MobileSeriesMembership>,
    pub saved_at_ms: u64,
    pub saved_version: i64,
    pub saved_content_hash: Option<String>,
    pub saved_edit_hash: Option<String>,
    pub page_count: usize,
    #[serde(default)]
    pub collections: Vec<MobileCollection>,
    #[serde(default)]
    pub collection_positions: BTreeMap<String, i64>,
    #[serde(default)]
    pub collection_cover_ids: Vec<String>,
    #[serde(default)]
    pub has_author_cover: bool,
    #[serde(default)]
    pub has_series_cover: bool,
    #[serde(default)]
    pub author_cover_entity: Option<OfflineEntityKey>,
    #[serde(default)]
    pub series_cover_entity: Option<OfflineEntityKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineEntityKey {
    pub source: String,
    pub key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrentPointer {
    snapshot: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageRecord {
    page: usize,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetRecord {
    id: i64,
    mime_type: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverRecord {
    mime_type: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollectionCoverRecord {
    collection_id: String,
    file_number: usize,
    mime_type: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    summary: OfflineWork,
    pages: Vec<PageRecord>,
    assets: Vec<AssetRecord>,
    #[serde(default)]
    cover: Option<CoverRecord>,
    #[serde(default)]
    author_cover: Option<CoverRecord>,
    #[serde(default)]
    series_cover: Option<CoverRecord>,
    #[serde(default)]
    collection_covers: Vec<CollectionCoverRecord>,
    #[serde(default)]
    outline_sha256: Option<String>,
}

fn storage() -> ClientError {
    ClientError::offline_storage()
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<Vec<u8>, ClientError> {
    let bytes = serde_json::to_vec(value).map_err(|_| storage())?;
    fs::write(path, &bytes).map_err(|_| storage())?;
    Ok(bytes)
}

fn read_manifest(state: &OfflineState, work_id: i64) -> Result<(Manifest, PathBuf), ClientError> {
    let work_dir = state.work_dir(work_id)?;
    let pointer: CurrentPointer =
        serde_json::from_slice(&fs::read(work_dir.join("current.json")).map_err(|_| storage())?)
            .map_err(|_| storage())?;
    let snapshot_id = pointer
        .snapshot
        .strip_prefix("snapshot-")
        .ok_or_else(storage)?;
    Uuid::parse_str(snapshot_id).map_err(|_| storage())?;
    let snapshot = work_dir.join(pointer.snapshot);
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(snapshot.join("manifest.json")).map_err(|_| storage())?)
            .map_err(|_| storage())?;
    if manifest.summary.work.id != work_id
        || manifest.pages.len() != manifest.summary.page_count
        || manifest.summary.page_count == 0
        || manifest.summary.page_count > MAX_PAGES
    {
        return Err(storage());
    }
    Ok((manifest, snapshot))
}

fn asset_ids(html: &str) -> BTreeSet<i64> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r#"data-asset-id\s*=\s*["']([0-9]+)["']"#).expect("valid asset pattern")
    });
    pattern
        .captures_iter(html)
        .filter_map(|capture| capture[1].parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect()
}

async fn get_pc(
    client: &reqwest::Client,
    saved: &SavedConnection,
    endpoint: &str,
) -> Result<reqwest::Response, ClientError> {
    get_pc_with_timeout(client, saved, endpoint, REQUEST_TIMEOUT).await
}

async fn get_pc_with_timeout(
    client: &reqwest::Client,
    saved: &SavedConnection,
    endpoint: &str,
    timeout: Duration,
) -> Result<reqwest::Response, ClientError> {
    let response = client
        .get(format!("{}{}", saved.url, endpoint))
        .bearer_auth(&saved.token)
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| ClientError::offline())?;
    check_response(response)
}

async fn save_entity_cover(
    client: &reqwest::Client,
    saved: &SavedConnection,
    source: &str,
    kind: &str,
    key: &str,
    staging: &Path,
    total_bytes: &mut usize,
) -> Result<Option<CoverRecord>, ClientError> {
    if key.is_empty() {
        return Ok(None);
    }
    let mut url = reqwest::Url::parse(&format!("{}/api/v1/entities/{kind}/cover", saved.url))
        .map_err(|_| ClientError::protocol())?;
    url.query_pairs_mut()
        .append_pair("source", source)
        .append_pair("key", key);
    let response = match client
        .get(url)
        .bearer_auth(&saved.token)
        .timeout(ASSET_TIMEOUT)
        .send()
        .await
    {
        Ok(response) => match check_response(response) {
            Ok(response) => response,
            Err(_) => return Ok(None),
        },
        Err(_) => return Ok(None),
    };
    let image = match image_response(response).await {
        Ok(image) => image,
        Err(_) => return Ok(None),
    };
    let bytes = STANDARD
        .decode(&image.base64)
        .map_err(|_| ClientError::protocol())?;
    if total_bytes.saturating_add(bytes.len()) > MAX_SNAPSHOT_BYTES {
        return Ok(None);
    }
    fs::write(staging.join(format!("{kind}-cover.bin")), &bytes).map_err(|_| storage())?;
    *total_bytes += bytes.len();
    Ok(Some(CoverRecord {
        mime_type: image.mime_type,
        sha256: digest(&bytes),
    }))
}

async fn save_collection_covers(
    client: &reqwest::Client,
    saved: &SavedConnection,
    collections: &[MobileCollection],
    staging: &Path,
    total_bytes: &mut usize,
) -> Result<Vec<CollectionCoverRecord>, ClientError> {
    let mut records = Vec::new();
    let mut cover_bytes = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    for collection in collections.iter().filter(|value| value.cover_mode == "file" && value.has_file_cover).take(MAX_COLLECTION_COVERS) {
        if cover_bytes >= MAX_COLLECTION_COVER_BYTES || tokio::time::Instant::now() >= deadline { break; }
        if collection.id.is_empty() || collection.id.len() > 128
            || !collection.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) { continue; }
        let endpoint = format!("/api/v1/collections/{}/cover/file", collection.id);
        let response = match tokio::time::timeout_at(deadline, get_pc_with_timeout(client, saved, &endpoint, ASSET_TIMEOUT)).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => continue,
            Err(_) => break,
        };
        let image = match image_response(response).await { Ok(image) => image, Err(_) => continue };
        let bytes = match STANDARD.decode(&image.base64) { Ok(bytes) => bytes, Err(_) => continue };
        if cover_bytes.saturating_add(bytes.len()) > MAX_COLLECTION_COVER_BYTES
            || total_bytes.saturating_add(bytes.len()) > MAX_SNAPSHOT_BYTES { continue; }
        let file_number = records.len();
        fs::write(staging.join(format!("collection-cover-{file_number}.bin")), &bytes).map_err(|_| storage())?;
        cover_bytes += bytes.len();
        *total_bytes += bytes.len();
        records.push(CollectionCoverRecord {
            collection_id: collection.id.clone(),
            file_number,
            mime_type: image.mime_type,
            sha256: digest(&bytes),
        });
    }
    Ok(records)
}

#[tauri::command]
pub async fn offline_list_works(
    state: State<'_, OfflineState>,
) -> Result<Vec<OfflineWork>, ClientError> {
    let _guard = state.lock.lock().await;
    let entries = match fs::read_dir(&state.root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(storage()),
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| storage())?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix("work-"))
            .and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        if !entry.file_type().map_err(|_| storage())?.is_dir() {
            continue;
        }
        match read_manifest(&state, id) {
            Ok((manifest, _)) => result.push(manifest.summary),
            Err(_) if !entry.path().join("current.json").exists() => continue,
            Err(error) => return Err(error),
        }
    }
    result.sort_by(|left, right| right.saved_at_ms.cmp(&left.saved_at_ms));
    Ok(result)
}

#[tauri::command]
pub async fn offline_save_work(
    pc: State<'_, PcClientState>,
    state: State<'_, OfflineState>,
    work_id: i64,
) -> Result<OfflineWork, ClientError> {
    // 通信と検証の間も、既存のスナップショットを読めるようにする。
    let _save_guard = state.save_lock.lock().await;
    let work_dir = state.work_dir(work_id)?;
    let saved = pc.load()?.ok_or_else(ClientError::invalid_pairing)?;
    let certificate = URL_SAFE_NO_PAD
        .decode(&saved.certificate_der)
        .map_err(|_| storage())?;
    let client = pinned_client(&certificate)?;
    let metadata: MobileMetadata = get_pc(&client, &saved, &format!("/api/v1/works/{work_id}"))
        .await?
        .json()
        .await
        .map_err(|_| ClientError::protocol())?;
    if metadata.work.id != work_id {
        return Err(ClientError::protocol());
    }
    fs::create_dir_all(&work_dir).map_err(|_| storage())?;
    let snapshot_name = format!("snapshot-{}", Uuid::new_v4());
    let staging = work_dir.join(format!("staging-{}", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|_| storage())?;
    let result = save_into(&client, &saved, &metadata, &staging).await;
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    // 公開中の版の切替だけ、読み込みと排他にする。
    let _guard = state.lock.lock().await;
    let pointer = publish_snapshot(&work_dir, &staging, snapshot_name)?;
    drop(_guard);
    // 新しい版への切替後だけ、以前の版を取り除く。
    if let Ok(entries) = fs::read_dir(&work_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("snapshot-") && name != pointer.snapshot {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(manifest.summary)
}

fn publish_snapshot(
    work_dir: &Path,
    staging: &Path,
    snapshot_name: String,
) -> Result<CurrentPointer, ClientError> {
    let snapshot = work_dir.join(&snapshot_name);
    if fs::rename(&staging, &snapshot).is_err() {
        let _ = fs::remove_dir_all(&staging);
        return Err(storage());
    }
    let pointer = CurrentPointer {
        snapshot: snapshot_name,
    };
    let temporary_pointer = work_dir.join("current.json.tmp");
    if write_json(&temporary_pointer, &pointer).is_err()
        || fs::rename(&temporary_pointer, work_dir.join("current.json")).is_err()
    {
        let _ = fs::remove_file(&temporary_pointer);
        let _ = fs::remove_dir_all(&snapshot);
        return Err(storage());
    }
    Ok(pointer)
}

async fn save_into(
    client: &reqwest::Client,
    saved: &SavedConnection,
    metadata: &MobileMetadata,
    staging: &Path,
) -> Result<Manifest, ClientError> {
    fs::create_dir(staging.join("pages")).map_err(|_| storage())?;
    fs::create_dir(staging.join("assets")).map_err(|_| storage())?;
    let work_id = metadata.work.id;
    let first: MobilePage = get_pc(client, saved, &format!("/api/v1/works/{work_id}/pages/0"))
        .await?
        .json()
        .await
        .map_err(|_| ClientError::protocol())?;
    if first.page != 0 || first.page_count == 0 || first.page_count > MAX_PAGES {
        return Err(ClientError::protocol());
    }
    let mut pages = Vec::with_capacity(first.page_count);
    let mut assets = BTreeSet::new();
    let mut total_bytes = 0_usize;
    for index in 0..first.page_count {
        let page: MobilePage = if index == 0 {
            first.clone()
        } else {
            get_pc(
                client,
                saved,
                &format!("/api/v1/works/{work_id}/pages/{index}"),
            )
            .await?
            .json()
            .await
            .map_err(|_| ClientError::protocol())?
        };
        if page.page != index || page.page_count != first.page_count {
            return Err(ClientError::protocol());
        }
        assets.extend(asset_ids(&page.html));
        if assets.len() > MAX_ASSETS {
            return Err(ClientError::remote());
        }
        let bytes = serde_json::to_vec(&page).map_err(|_| storage())?;
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(ClientError::remote());
        }
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(ClientError::remote)?;
        if total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(ClientError::remote());
        }
        fs::write(staging.join("pages").join(format!("{index}.json")), &bytes)
            .map_err(|_| storage())?;
        pages.push(PageRecord {
            page: index,
            sha256: digest(&bytes),
        });
    }
    let mut asset_records = Vec::with_capacity(assets.len());
    for id in assets {
        let response = get_pc_with_timeout(
            client,
            saved,
            &format!("/api/v1/works/{work_id}/assets/{id}"),
            ASSET_TIMEOUT,
        )
        .await?;
        let mime_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(ClientError::protocol)?
            .to_string();
        if !matches!(
            mime_type.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif" | "image/avif"
        ) {
            return Err(ClientError::protocol());
        }
        let bytes = response.bytes().await.map_err(|_| ClientError::offline())?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err(ClientError::remote());
        }
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(ClientError::remote)?;
        if total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(ClientError::remote());
        }
        fs::write(staging.join("assets").join(format!("{id}.bin")), &bytes)
            .map_err(|_| storage())?;
        asset_records.push(AssetRecord {
            id,
            mime_type,
            sha256: digest(&bytes),
        });
    }
    // 表紙は本文の画像一覧とは別に保管する。取得できなくても本文の保存は続ける。
    let mut cover = None;
    if metadata.work.has_cover {
        if let Ok(response) = get_pc_with_timeout(
            client,
            saved,
            &format!("/api/v1/works/{work_id}/cover"),
            ASSET_TIMEOUT,
        )
        .await
        {
            if let Ok(image) = image_response(response).await {
                if let Ok(bytes) = STANDARD.decode(&image.base64) {
                    if total_bytes.saturating_add(bytes.len()) <= MAX_SNAPSHOT_BYTES {
                        fs::write(staging.join("cover.bin"), &bytes).map_err(|_| storage())?;
                        total_bytes += bytes.len();
                        cover = Some(CoverRecord {
                            mime_type: image.mime_type,
                            sha256: digest(&bytes),
                        });
                    }
                }
            }
        }
    }
    let author_cover = save_entity_cover(
        client,
        saved,
        &metadata.work.source,
        "people",
        &metadata.work.author_id,
        staging,
        &mut total_bytes,
    )
    .await?;
    let series_cover = if let Some(series_id) = metadata.work.series_id.as_deref() {
        save_entity_cover(
            client,
            saved,
            &metadata.work.source,
            "series",
            series_id,
            staging,
            &mut total_bytes,
        )
        .await?
    } else {
        None
    };
    let mut outline_sha256 = None;
    if let Ok(response) = get_pc(client, saved, &format!("/api/v1/works/{work_id}/outline")).await {
        if let Ok(entries) = response.json::<Vec<MobileOutlineEntry>>().await {
            if entries.len() <= 2_000 {
                let bytes = serde_json::to_vec(&entries).map_err(|_| storage())?;
                if bytes.len() <= 1_000_000 {
                    fs::write(staging.join("outline.json"), &bytes).map_err(|_| storage())?;
                    outline_sha256 = Some(digest(&bytes));
                }
            }
        }
    }
    let collections: Vec<MobileCollection> = get_pc(
        client,
        saved,
        &format!("/api/v1/works/{work_id}/collections"),
    )
    .await?
    .json()
    .await
    .map_err(|_| ClientError::protocol())?;
    let collection_positions = collections.iter()
        .filter_map(|collection| collection.position.map(|position| (collection.id.clone(), position)))
        .collect::<BTreeMap<_, _>>();
    let collection_covers = save_collection_covers(client, saved, &collections, staging, &mut total_bytes).await?;
    let collection_cover_ids = collection_covers.iter().map(|record| record.collection_id.clone()).collect();
    // 読み込み中に PC 側が更新された場合、混在した版を保存しない。
    let latest: MobileMetadata = get_pc(client, saved, &format!("/api/v1/works/{work_id}"))
        .await?
        .json()
        .await
        .map_err(|_| ClientError::protocol())?;
    if latest.work.current_version != metadata.work.current_version
        || latest.work.content_hash != metadata.work.content_hash
        || latest.active_edit_hash != metadata.active_edit_hash
    {
        return Err(ClientError::remote());
    }
    let summary = OfflineWork {
        work: metadata.work.clone(),
        people: metadata.people.clone(),
        series: metadata.series.clone(),
        saved_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| storage())?
            .as_millis() as u64,
        saved_version: metadata.work.current_version,
        saved_content_hash: metadata.work.content_hash.clone(),
        saved_edit_hash: metadata.active_edit_hash.clone(),
        page_count: pages.len(),
        collections,
        collection_positions,
        collection_cover_ids,
        has_author_cover: author_cover.is_some(),
        has_series_cover: series_cover.is_some(),
        author_cover_entity: author_cover.as_ref().map(|_| OfflineEntityKey {
            source: metadata.work.source.clone(),
            key: metadata.work.author_id.clone(),
        }),
        series_cover_entity: series_cover.as_ref().and_then(|_| metadata.work.series_id.as_ref().map(|key| OfflineEntityKey {
            source: metadata.work.source.clone(),
            key: key.clone(),
        })),
    };
    let manifest = Manifest {
        summary,
        pages,
        assets: asset_records,
        cover,
        author_cover,
        series_cover,
        collection_covers,
        outline_sha256,
    };
    write_json(&staging.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

#[tauri::command]
pub async fn offline_get_outline(
    state: State<'_, OfflineState>,
    work_id: i64,
) -> Result<Vec<MobileOutlineEntry>, ClientError> {
    let _guard = state.lock.lock().await;
    let (manifest, snapshot) = read_manifest(&state, work_id)?;
    let Some(expected) = manifest.outline_sha256 else {
        return Ok(Vec::new());
    };
    let bytes = fs::read(snapshot.join("outline.json")).map_err(|_| storage())?;
    if bytes.len() > 1_000_000 || digest(&bytes) != expected {
        return Err(storage());
    }
    serde_json::from_slice(&bytes).map_err(|_| storage())
}

#[tauri::command]
pub async fn offline_get_cover(
    state: State<'_, OfflineState>,
    work_id: i64,
) -> Result<MobileAssetData, ClientError> {
    let _guard = state.lock.lock().await;
    let (manifest, snapshot) = read_manifest(&state, work_id)?;
    let record = manifest.cover.ok_or_else(ClientError::remote)?;
    let bytes = fs::read(snapshot.join("cover.bin")).map_err(|_| storage())?;
    if bytes.len() > MAX_ASSET_BYTES || digest(&bytes) != record.sha256 {
        return Err(storage());
    }
    Ok(MobileAssetData {
        mime_type: record.mime_type,
        base64: STANDARD.encode(bytes),
    })
}

#[tauri::command]
pub async fn offline_get_collection_cover(
    state: State<'_, OfflineState>,
    work_id: i64,
    collection_id: String,
) -> Result<MobileAssetData, ClientError> {
    let _guard = state.lock.lock().await;
    let (manifest, snapshot) = read_manifest(&state, work_id)?;
    let record = manifest.collection_covers.iter()
        .find(|record| record.collection_id == collection_id)
        .ok_or_else(ClientError::remote)?;
    let bytes = fs::read(snapshot.join(format!("collection-cover-{}.bin", record.file_number))).map_err(|_| storage())?;
    if bytes.len() > MAX_ASSET_BYTES || digest(&bytes) != record.sha256 { return Err(storage()); }
    Ok(MobileAssetData { mime_type: record.mime_type.clone(), base64: STANDARD.encode(bytes) })
}

#[tauri::command]
pub async fn offline_get_entity_cover(
    state: State<'_, OfflineState>,
    work_id: i64,
    kind: String,
) -> Result<MobileAssetData, ClientError> {
    let _guard = state.lock.lock().await;
    let (manifest, snapshot) = read_manifest(&state, work_id)?;
    let (record, filename) = match kind.as_str() {
        "people" => (manifest.author_cover.as_ref(), "people-cover.bin"),
        "series" => (manifest.series_cover.as_ref(), "series-cover.bin"),
        _ => return Err(ClientError::remote()),
    };
    let record = record.ok_or_else(ClientError::remote)?;
    let bytes = fs::read(snapshot.join(filename)).map_err(|_| storage())?;
    if bytes.len() > MAX_ASSET_BYTES || digest(&bytes) != record.sha256 {
        return Err(storage());
    }
    Ok(MobileAssetData {
        mime_type: record.mime_type.clone(),
        base64: STANDARD.encode(bytes),
    })
}

#[tauri::command]
pub async fn offline_get_page(
    state: State<'_, OfflineState>,
    work_id: i64,
    page: usize,
) -> Result<MobilePage, ClientError> {
    let _guard = state.lock.lock().await;
    let (manifest, snapshot) = read_manifest(&state, work_id)?;
    let page = page.min(manifest.summary.page_count - 1);
    let record = manifest
        .pages
        .get(page)
        .filter(|record| record.page == page)
        .ok_or_else(ClientError::remote)?;
    let bytes =
        fs::read(snapshot.join("pages").join(format!("{page}.json"))).map_err(|_| storage())?;
    if bytes.len() > MAX_PAGE_BYTES || digest(&bytes) != record.sha256 {
        return Err(storage());
    }
    let page_data: MobilePage = serde_json::from_slice(&bytes).map_err(|_| storage())?;
    if page_data.page != page || page_data.page_count != manifest.summary.page_count {
        return Err(storage());
    }
    Ok(page_data)
}

#[tauri::command]
pub async fn offline_get_asset(
    state: State<'_, OfflineState>,
    work_id: i64,
    asset_id: i64,
) -> Result<MobileAssetData, ClientError> {
    let _guard = state.lock.lock().await;
    let (manifest, snapshot) = read_manifest(&state, work_id)?;
    let record = manifest
        .assets
        .iter()
        .find(|record| record.id == asset_id)
        .ok_or_else(ClientError::remote)?;
    let bytes =
        fs::read(snapshot.join("assets").join(format!("{asset_id}.bin"))).map_err(|_| storage())?;
    if bytes.len() > MAX_ASSET_BYTES || digest(&bytes) != record.sha256 {
        return Err(storage());
    }
    Ok(MobileAssetData {
        mime_type: record.mime_type.clone(),
        base64: STANDARD.encode(bytes),
    })
}

#[tauri::command]
pub async fn offline_delete_work(
    state: State<'_, OfflineState>,
    work_id: i64,
) -> Result<(), ClientError> {
    let _save_guard = state.save_lock.lock().await;
    let _guard = state.lock.lock().await;
    let dir = state.work_dir(work_id)?;
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(storage()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memberships_survive_snapshot_and_legacy_snapshot_defaults_to_singular_fields() {
        let work = serde_json::json!({
            "id": 7, "source": "pixiv", "sourceId": "work-7", "title": "作品",
            "authorName": "主作者", "authorId": "main-author", "contentType": "novel",
            "tags": [], "excerpt": null, "textLength": 10, "assetCount": 0,
            "hasCover": false, "currentVersion": 1, "contentHash": null,
            "favorite": false, "seriesTitle": "主シリーズ", "seriesId": "main-series",
            "downloadedAt": "2026-10-10T00:00:00Z"
        });
        let people = serde_json::json!([
            {"source": "pixiv", "key": "main-author", "role": "author", "displayName": "主作者"},
            {"source": "pixiv", "key": "co-author", "role": "author", "displayName": "共作者"}
        ]);
        let series = serde_json::json!([
            {"source": "pixiv", "key": "main-series", "title": "主シリーズ", "contentOrder": 1},
            {"source": "pixiv", "key": "anthology", "title": "短編集", "contentOrder": null}
        ]);
        let metadata_json = serde_json::json!({
            "work": work, "people": people, "series": series,
            "versions": [], "isEdited": false, "activeEditId": null, "activeEditHash": null
        });
        let metadata: MobileMetadata = serde_json::from_value(metadata_json.clone()).unwrap();
        assert_eq!(metadata.people.len(), 2);
        assert_eq!(metadata.series.len(), 2);
        let mut old_response = metadata_json;
        old_response.as_object_mut().unwrap().remove("people");
        old_response.as_object_mut().unwrap().remove("series");
        let old_metadata: MobileMetadata = serde_json::from_value(old_response).unwrap();
        assert!(old_metadata.people.is_empty());
        assert!(old_metadata.series.is_empty());

        let snapshot = serde_json::json!({
            "work": metadata.work, "people": metadata.people, "series": metadata.series,
            "savedAtMs": 1, "savedVersion": 1, "savedContentHash": null,
            "savedEditHash": null, "pageCount": 1,
            "hasAuthorCover": true, "hasSeriesCover": true,
            "authorCoverEntity": {"source": "pixiv", "key": "co-author"},
            "seriesCoverEntity": {"source": "pixiv", "key": "anthology"}
        });
        let saved: OfflineWork = serde_json::from_value(snapshot.clone()).unwrap();
        assert_eq!(saved.people[1].key, "co-author");
        assert_eq!(saved.series[1].key, "anthology");
        assert!(saved.series[1].content_order.is_none());
        assert_eq!(saved.author_cover_entity.as_ref().unwrap().key, "co-author");
        assert_eq!(saved.series_cover_entity.as_ref().unwrap().key, "anthology");
        let round_trip: OfflineWork = serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
        assert_eq!(round_trip.people.len(), 2);
        assert_eq!(round_trip.series.len(), 2);
        assert_eq!(round_trip.author_cover_entity.as_ref().unwrap().source, "pixiv");

        let mut legacy = snapshot;
        legacy.as_object_mut().unwrap().remove("people");
        legacy.as_object_mut().unwrap().remove("series");
        legacy.as_object_mut().unwrap().remove("authorCoverEntity");
        legacy.as_object_mut().unwrap().remove("seriesCoverEntity");
        let old_saved: OfflineWork = serde_json::from_value(legacy).unwrap();
        assert!(old_saved.people.is_empty());
        assert!(old_saved.series.is_empty());
        assert!(old_saved.author_cover_entity.is_none());
        assert!(old_saved.series_cover_entity.is_none());
        assert_eq!(old_saved.work.author_id, "main-author");
        assert_eq!(old_saved.work.series_id.as_deref(), Some("main-series"));
    }

    #[test]
    fn extracts_only_numeric_image_references() {
        let ids = asset_ids(
            r#"<img data-asset-id="42"><img data-asset-id='7'><img src="file://private"><img data-asset-id="42">"#,
        );
        assert_eq!(ids, BTreeSet::from([7, 42]));
    }

    #[test]
    fn corrupt_snapshot_is_rejected_without_pc_connection() {
        let root = std::env::temp_dir().join(format!("piep-offline-test-{}", Uuid::new_v4()));
        let state = OfflineState::new(root.clone());
        let dir = state.work_dir(3).unwrap();
        fs::create_dir_all(&dir).unwrap();
        write_json(
            &dir.join("current.json"),
            &CurrentPointer {
                snapshot: "../private".into(),
            },
        )
        .unwrap();
        assert!(read_manifest(&state, 3).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pointer_write_failure_preserves_old_snapshot_and_removes_new_one() {
        let root = std::env::temp_dir().join(format!("piep-offline-test-{}", Uuid::new_v4()));
        let work_dir = root.join("work-3");
        let old_name = format!("snapshot-{}", Uuid::new_v4());
        let new_name = format!("snapshot-{}", Uuid::new_v4());
        let staging = work_dir.join(format!("staging-{}", Uuid::new_v4()));
        fs::create_dir_all(work_dir.join(&old_name)).unwrap();
        fs::create_dir(&staging).unwrap();
        write_json(
            &work_dir.join("current.json"),
            &CurrentPointer {
                snapshot: old_name.clone(),
            },
        )
        .unwrap();
        // 書き込み不能を同名のディレクトリで再現する。
        fs::create_dir(work_dir.join("current.json.tmp")).unwrap();
        assert!(publish_snapshot(&work_dir, &staging, new_name.clone()).is_err());
        let current: CurrentPointer =
            serde_json::from_slice(&fs::read(work_dir.join("current.json")).unwrap()).unwrap();
        assert_eq!(current.snapshot, old_name);
        assert!(work_dir.join(&old_name).is_dir());
        assert!(!work_dir.join(new_name).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
