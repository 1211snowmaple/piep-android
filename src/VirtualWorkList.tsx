import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

/** Keep only the card rows near the main scroll viewport mounted. */
export function VirtualWorkList<T>({ items, itemKey, renderItem }: {
  items: T[];
  itemKey: (item: T) => string | number;
  renderItem: (item: T) => ReactNode;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  const [scroller, setScroller] = useState<HTMLElement | null>(() => document.querySelector<HTMLElement>(".main-content"));
  const [width, setWidth] = useState(0);
  const [scrollMargin, setScrollMargin] = useState(0);
  const mobile = typeof window !== "undefined" && window.matchMedia("(max-width: 720px)").matches;
  const gap = mobile ? 10 : 14;
  const columns = mobile ? 1 : Math.max(1, Math.floor((width + gap) / (275 + gap)));

  useLayoutEffect(() => {
    const node = listRef.current;
    if (!node) return;
    const scrollNode = node.closest<HTMLElement>(".main-content");
    setScroller(scrollNode);
    const measure = () => {
      if (node.clientWidth > 0) setWidth((old) => old === node.clientWidth ? old : node.clientWidth);
      if (scrollNode) {
        const margin = node.getBoundingClientRect().top - scrollNode.getBoundingClientRect().top + scrollNode.scrollTop;
        setScrollMargin((old) => Math.abs(old - margin) < 0.5 ? old : margin);
      }
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (scrollNode) observer.observe(scrollNode);
    return () => observer.disconnect();
  }, []);

  const virtualizer = useVirtualizer({
    count: Math.ceil(items.length / columns),
    getScrollElement: () => scroller,
    enabled: Boolean(scroller),
    initialOffset: () => scroller?.scrollTop ?? 0,
    estimateSize: () => mobile ? 170 : 220,
    getItemKey: (index) => `${columns}:${items[index * columns] ? itemKey(items[index * columns]) : index}`,
    gap,
    overscan: mobile ? 6 : 3,
    scrollMargin,
  });

  // The layout width is unknown for one paint on first mount. A bounded grid
  // gives the user immediate content without mounting an unbounded library.
  if (!scroller || width <= 0) {
    return <div className="work-list" ref={listRef}>{items.slice(0, 20).map((item) => <div key={itemKey(item)}>{renderItem(item)}</div>)}</div>;
  }

  return <div ref={listRef} className="virtual-work-list" style={{ height: virtualizer.getTotalSize() }}>
    {virtualizer.getVirtualItems().map((row) => <div
      className="virtual-work-row"
      key={row.key}
      ref={virtualizer.measureElement}
      data-index={row.index}
      style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`, transform: `translateY(${row.start - scrollMargin}px)` }}
    >{items.slice(row.index * columns, (row.index + 1) * columns).map((item) => <div className="virtual-work-cell" key={itemKey(item)}>{renderItem(item)}</div>)}</div>)}
  </div>;
}
