import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getOfflineCover, getPcCover, type PcWork } from "./pcApi";
import { coverSigil, titleScale } from "./coverSigil";
import { DataUrlCache } from "./DataUrlCache";

const cache = new DataUrlCache(16 * 1024 * 1024);
const invalidCovers = new Set<string>();
const pendingCovers = new Map<string, Promise<string | null>>();
const waiting: (() => void)[] = [];
let active = 0;

export function enqueueCoverRequest<T>(work: () => Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const run = () => {
      active += 1;
      void work().then(resolve, reject).finally(() => {
        active -= 1;
        waiting.shift()?.();
      });
    };
    if (active < 4) run();
    else waiting.push(run);
  });
}

export function WorkThumbnail({ work, offline = false, snapshotGeneration, variant = "card" }: { work: PcWork; offline?: boolean; snapshotGeneration?: number; variant?: "card" | "detail" }) {
  const node = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(false);
  const key = `${offline ? `saved:${snapshotGeneration ?? 0}` : "pc"}:${work.id}:${work.currentVersion}:${work.contentHash ?? ""}`;
  const [source, setSource] = useState<string | null>(null);
  const [ratio, setRatio] = useState<number | null>(null);
  const sigil = useMemo(() => coverSigil(work), [work.source, work.sourceId, work.authorName]);

  useEffect(() => {
    setSource(null);
    setRatio(null);
    setVisible(false);
    const target = node.current;
    if (!target || !work.hasCover || !isTauri()) return;
    if (typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => {
      setVisible(entries.some((entry) => entry.isIntersecting));
    }, { rootMargin: "240px" });
    observer.observe(target);
    return () => observer.disconnect();
  }, [key, work.hasCover]);

  useEffect(() => {
    if (!visible || !work.hasCover || invalidCovers.has(key)) { setSource(null); return; }
    if (source) return;
    const cached = cache.get(key);
    if (cached) { setSource(cached); return; }
    let alive = true;
    let request = pendingCovers.get(key);
    if (!request) {
      request = enqueueCoverRequest(() => offline ? getOfflineCover(work.id) : getPcCover(work.id))
        .then((image) => {
        const url = `data:${image.mimeType};base64,${image.base64}`;
        cache.set(key, url);
        return url;
      })
        .catch(() => null)
        .finally(() => { pendingCovers.delete(key); });
      pendingCovers.set(key, request);
    }
    void request.then((url) => { if (alive && url) setSource(url); });
    return () => { alive = false; };
  }, [key, visible, work.hasCover, work.id, offline, source]);

  return <span className="work-cover" ref={node} data-image={source ? "true" : undefined} data-detail={variant === "detail" ? "true" : undefined} style={{ "--work-cover-hue": sigil.hue, "--work-cover-accent": work.source === "fanbox" ? "#f2c624" : "#0096fa", ...(variant === "detail" && ratio ? { "--work-cover-ratio": ratio } : {}) } as CSSProperties}>
    {source ? <img className="work-cover-image" src={source} alt={`${work.title}の表紙`} loading={variant === "detail" ? "eager" : "lazy"} decoding="async" onLoad={(event) => { const image = event.currentTarget; if (image.naturalWidth > 0 && image.naturalHeight > 0) setRatio(image.naturalWidth / image.naturalHeight); }} onError={() => { invalidCovers.add(key); cache.delete(key); setSource(null); setRatio(null); }} /> : <span className="cover-empty" role="img" aria-label={`${work.title}（表紙なし）`}>
      <span className="cover-sigil" aria-hidden="true">{sigil.cells.map((filled, index) => <i key={index} data-on={filled || undefined} />)}</span>
      <span className="cover-caption">{work.seriesTitle && <small>{work.seriesTitle}</small>}<span data-scale={titleScale(work.title)}>{work.title}</span></span>
    </span>}
  </span>;
}
