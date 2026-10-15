import { useEffect, useRef, useState, type CSSProperties } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { authorHue, sigilCells } from "./coverSigil";
import { DataUrlCache } from "./DataUrlCache";
import { getOfflineCollectionCover, getOfflineCover, getPcCollectionCover, getPcCover, type OfflineWork, type PcAsset, type PcCollection } from "./pcApi";
import { enqueueCoverRequest } from "./WorkThumbnail";

const imageCache = new DataUrlCache(16 * 1024 * 1024);
const failedImages = new Set<string>();
const pendingImages = new Map<string, Promise<string>>();

function loadImage(cacheKey: string, load: () => Promise<PcAsset>): Promise<string> {
  const existing = pendingImages.get(cacheKey);
  if (existing) return existing;
  const request = enqueueCoverRequest(load).then((asset) => {
    const url = `data:${asset.mimeType};base64,${asset.base64}`;
    imageCache.set(cacheKey, url);
    return url;
  }).finally(() => { pendingImages.delete(cacheKey); });
  pendingImages.set(cacheKey, request);
  return request;
}

function CoverImage({ cacheKey, visible, load, alt, onFailed }: {
  cacheKey: string;
  visible: boolean;
  load: () => Promise<PcAsset>;
  alt: string;
  onFailed?: () => void;
}) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    setUrl(null);
    if (!visible || !isTauri()) return;
    if (failedImages.has(cacheKey)) { onFailed?.(); return; }
    const cached = imageCache.get(cacheKey);
    if (cached) { setUrl(cached); return; }
    let alive = true;
    void loadImage(cacheKey, load).then((next) => {
      if (alive) setUrl(next);
    }).catch(() => {
      if (alive) onFailed?.();
    });
    return () => { alive = false; };
  }, [cacheKey, visible]);
  return url && visible ? <><img className="collection-art-backdrop" src={url} alt="" aria-hidden="true" /><img className="collection-art-image" src={url} alt={alt} decoding="async" onError={() => { failedImages.add(cacheKey); imageCache.delete(cacheKey); setUrl(null); onFailed?.(); }} /></> : null;
}

function Sigil({ seed, hue }: { seed: string; hue: number }) {
  return <span className="collection-art-sigil" style={{ "--work-cover-hue": hue } as CSSProperties} aria-hidden="true">{sigilCells(seed).map((filled, index) => <i key={index} data-on={filled || undefined} />)}</span>;
}

function Tile({ collection, index, visible, offlineWorks }: { collection: PcCollection; index: number; visible: boolean; offlineWorks?: OfflineWork[] }) {
  const tile = collection.coverTiles![index];
  const saved = offlineWorks?.find((item) => item.work.source === tile.source && item.work.sourceId === tile.sourceId && item.work.hasCover);
  const key = `${offlineWorks ? `saved:${saved?.work.id ?? 0}:${saved?.savedAtMs ?? 0}` : "pc"}:${collection.id}:${collection.coverRevision ?? 0}:tile-${index}`;
  return <span className="collection-art-tile" style={{ "--slot-index": index } as CSSProperties}>
    <span className="collection-art-tile-fallback" style={{ "--work-cover-hue": authorHue(tile.authorName || tile.source) } as CSSProperties}>
      <Sigil seed={`${tile.source}:${tile.sourceId}`} hue={authorHue(tile.authorName || tile.source)} />
    </span>
    {tile.hasCover && (!offlineWorks || saved) && <CoverImage cacheKey={key} visible={visible} load={() => saved ? getOfflineCover(saved.work.id) : getPcCollectionCover(collection.id, `tile-${index as 0 | 1 | 2 | 3}`)} alt="" />}
  </span>;
}

export function CollectionArtwork({ collection, offlineWorks }: { collection: PcCollection; offlineWorks?: OfflineWork[] }) {
  const node = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(false);
  const [mainFailed, setMainFailed] = useState(false);
  const tiles = collection.coverTiles?.slice(0, 4) ?? [];
  const requested = collection.coverMode ?? (collection.coverWorkId ? "single" : "sigil");
  const savedMain = offlineWorks?.find((item) => item.work.id === collection.coverWorkId && item.work.hasCover);
  const savedFile = offlineWorks?.find((item) => item.collectionCoverIds?.includes(collection.id)
    && item.collections?.some((entry) => entry.id === collection.id && entry.coverRevision === collection.coverRevision));
  const hasMain = (requested === "file" && collection.hasFileCover && (!offlineWorks || savedFile))
    || (requested === "single" && collection.hasSingleCover && collection.coverWorkId && (!offlineWorks || savedMain));
  const main = hasMain && !mainFailed;
  // A tile without an image still has its own work sigil, as on the desktop
  // shelf. Preserve the collection's chosen arrangement when tiles exist.
  const mode = main ? requested : requested === "sigil" || tiles.length === 0 ? "sigil" : requested === "spine" && tiles.length >= 2 ? "spine" : "mosaic";
  const key = `${offlineWorks ? `saved:${savedFile?.work.id ?? savedMain?.work.id ?? 0}:${savedFile?.savedAtMs ?? savedMain?.savedAtMs ?? 0}` : "pc"}:${collection.id}:${collection.coverRevision ?? 0}:${requested}:${collection.coverWorkId ?? 0}`;

  useEffect(() => { setMainFailed(false); }, [key]);
  useEffect(() => { if (visible) setMainFailed(false); }, [visible, key]);
  useEffect(() => {
    const target = node.current;
    if (!target) return;
    if (typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => setVisible(entries.some((entry) => entry.isIntersecting)), { rootMargin: "180px" });
    observer.observe(target);
    return () => observer.disconnect();
  }, []);

  return <span ref={node} className={`collection-art collection-art--${mode}`} data-tiles={tiles.length} role="img" aria-label={`${collection.name}の表紙`} style={{ "--work-cover-hue": authorHue(collection.name || collection.id) } as CSSProperties}>
    {main ? <CoverImage cacheKey={key} visible={visible} load={() => requested === "file" ? savedFile ? getOfflineCollectionCover(savedFile.work.id, collection.id) : getPcCollectionCover(collection.id, "file") : savedMain ? getOfflineCover(savedMain.work.id) : getPcCover(collection.coverWorkId!)} alt="" onFailed={() => setMainFailed(true)} />
      : mode === "sigil" ? <Sigil seed={`collection:${collection.id}`} hue={authorHue(collection.name || collection.id)} />
        : tiles.map((_, index) => <Tile key={`${collection.id}:${index}`} collection={collection} index={index} visible={visible} offlineWorks={offlineWorks} />)}
  </span>;
}
